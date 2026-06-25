import Foundation
import CoreBluetooth
import CryptoKit
import CommonCrypto

// MicroTech AiDEX X / GX-01S BLE probe (macOS / CoreBluetooth).
//
// Verified protocol (decompiled libblecomm + live device):
//   Service 181F.  F001 = pair (write/notify, used while unpaired).
//   F003 = reconnect notify (used once paired).  F002 = DevComm2 data (read/wwr/notify).
//   Key seed  = MD5( (base36(snChar)*13 + 61) mod 256 )   -- local only, written during pairing.
//   IV        = MD5( (base36(snChar)*17 + 19) mod 256 )    -- fixed per serial.
//   Pair key  = 16 bytes RETURNED by the device during pairing (NOT derivable from SN). Persist it.
//   Reconnect : read F002 -> 17-byte blob -> AES-128-CFB128 decrypt(pairKey, IV) ->
//               first 16 bytes = session key, byte 16 = CRC8/MAXIM(sessionKey).
//   DevComm2  : plaintext = [cmd][payload][CRC16-CCITT LE], AES-128-CFB128(sessionKey, IV); write F002.
//   Opcodes   : 0x10 deviceInfo, 0x11 broadcast/glucose, 0x20 newSensor(+9B datetime),
//               0x21 getStartTime, 0x22 historyRange, 0x23 histories, 0x24 rawHistories,
//               0x25 calibration, 0x34 setAutoUpdate, 0x35 setDynamicAdv.
//
// Not for treatment decisions.

private func hex(_ data: Data?) -> String {
    guard let data else { return "" }
    return data.map { String(format: "%02X", $0) }.joined()
}

private func crc8Maxim(_ bytes: [UInt8], initial: UInt8 = 0) -> UInt8 {
    var crc = initial
    for byte in bytes {
        crc ^= byte
        for _ in 0..<8 {
            crc = (crc & 1) != 0 ? (crc >> 1) ^ 0x8C : crc >> 1
        }
    }
    return crc
}

private func crc16CCITT(_ bytes: [UInt8], initial: UInt16 = 0xFFFF) -> UInt16 {
    var crc = initial
    for byte in bytes {
        crc ^= UInt16(byte) << 8
        for _ in 0..<8 {
            crc = (crc & 0x8000) != 0 ? (crc << 1) ^ 0x1021 : crc << 1
        }
    }
    return crc
}

private func leU16(_ bytes: [UInt8], at offset: Int) -> UInt16 {
    UInt16(bytes[offset]) | (UInt16(bytes[offset + 1]) << 8)
}

private func leU32(_ bytes: [UInt8], at offset: Int) -> UInt32 {
    UInt32(bytes[offset])
        | (UInt32(bytes[offset + 1]) << 8)
        | (UInt32(bytes[offset + 2]) << 16)
        | (UInt32(bytes[offset + 3]) << 24)
}

private func aidexAdvertisementCRC32(_ bytes: ArraySlice<UInt8>, initial: UInt32) -> UInt32 {
    var crc = initial
    for byte in bytes {
        crc ^= UInt32(byte) << 24
        for _ in 0..<8 {
            crc = (crc & 0x8000_0000) != 0 ? (crc &<< 1) ^ 0x04C1_1DB7 : crc &<< 1
        }
    }
    return crc
}

// MARK: - Broadcast / glucose record decoding

private struct AidexHistory {
    let timeOffset: UInt16
    let glucoseMgDl: UInt16
    let recordStatus: UInt8
    let quality: UInt8
}

private struct AidexBroadcast {
    let timeOffset: UInt16     // minutes since sensor start
    let status: UInt8
    let calTemp: UInt8
    let trend: Int8            // signed mg/dL rate
    let calIndex: UInt16
    let histories: [AidexHistory]
}

// Decode the 16-byte (advertisement) / N-byte (command-response) broadcast body:
//   [0,1] timeOffset LE, [2] status, [3] calTemp, [4] trend,
//   then 3-byte history slots {glucoseWord LE, quality}, then [N-2,N-1] calIndex LE.
private func decodeBroadcastBody(_ packet: [UInt8]) -> AidexBroadcast? {
    guard packet.count >= 10, (packet.count - 7).isMultiple(of: 3) else { return nil }
    let timeOffset = leU16(packet, at: 0)
    var histories: [AidexHistory] = []
    let slots = (packet.count - 7) / 3
    for index in 0..<slots {
        let off = 5 + index * 3
        let encoded = leU16(packet, at: off)
        guard encoded != 0xFFFF else { continue }   // 0xFFFF = no reading
        histories.append(
            AidexHistory(
                timeOffset: timeOffset &- UInt16(index),
                glucoseMgDl: encoded & 0x03FF,       // raw 10-bit value is mg/dL; mmol/L = /18
                recordStatus: UInt8((encoded >> 10) & 0x03),
                quality: packet[off + 2]
            )
        )
    }
    return AidexBroadcast(
        timeOffset: timeOffset,
        status: packet[2],
        calTemp: packet[3],
        trend: Int8(bitPattern: packet[4]),
        calIndex: leU16(packet, at: packet.count - 2),
        histories: histories
    )
}

private func decodeAidexAdvertisement(_ manufacturer: Data) -> (broadcast: AidexBroadcast, crcValid: Bool, nativePaired: Bool?, aesInitialized: Bool?)? {
    let bytes = [UInt8](manufacturer)
    guard bytes.count >= 22, bytes[0] == 0x59, bytes[1] == 0x00 else { return nil }
    let packet = Array(bytes[2..<22])
    var seedSum: UInt32 = 0
    for off in stride(from: 0, to: 16, by: 4) { seedSum = seedSum &+ leU32(packet, at: off) }
    let seed = seedSum % 0x007F_A777
    let actualCRC = aidexAdvertisementCRC32(packet[0..<16], initial: seed)
    let expectedCRC = leU32(packet, at: 16)
    guard let body = decodeBroadcastBody(Array(packet[0..<16])) else { return nil }
    let flags = bytes.count > 22 ? bytes[22] : nil
    return (body, actualCRC == expectedCRC,
            flags.map { ($0 & 0x01) != 0 }, flags.map { ($0 & 0x02) != 0 })
}

private func describeBroadcast(_ b: AidexBroadcast, crcValid: Bool? = nil, nativePaired: Bool? = nil, aesInitialized: Bool? = nil) -> String {
    let glucose = b.histories.isEmpty
        ? "glucose=none(noActiveReading)"
        : b.histories.map {
            let note = $0.glucoseMgDl >= 0x3FF ? "(saturated/warmup-invalid)"
                     : (glucoseIsValid(b, $0) ? "" : "(flagged-invalid)")
            return String(format: "glucoseMgDl=%d(%.1fmmol/L)%@ timeOffsetMin=%d recStatus=%d quality=%d valid=%@",
                          $0.glucoseMgDl, Double($0.glucoseMgDl) / 18.0, note,
                          $0.timeOffset, $0.recordStatus, $0.quality,
                          glucoseIsValid(b, $0) ? "YES" : "no")
        }.joined(separator: " ; ")
    let warmup = b.timeOffset < 60 ? " (WARMUP \(b.timeOffset)/60min)" : ""
    var parts: [String] = []
    if let crcValid { parts.append("crc=\(crcValid ? "valid" : "INVALID")") }
    parts.append("timeOffset=\(b.timeOffset)\(warmup)")
    parts.append(String(format: "status=0x%02X calTemp=0x%02X", b.status, b.calTemp))
    parts.append("trend=\(b.trend) calIndex=\(b.calIndex)")
    if let nativePaired { parts.append("nativePaired=\(nativePaired)") }
    if let aesInitialized { parts.append("aesInitialized=\(aesInitialized)") }
    parts.append(describeSensorState(status: b.status, calTemp: b.calTemp, timeOffset: b.timeOffset))
    parts.append(glucose)
    return parts.joined(separator: " ")
}

// A reading is a legit glucose value only after warmup, with a clean record/status and a
// physiological value (0x3FF/1023 is the saturated warmup-invalid marker, not a reading).
private func glucoseIsValid(_ b: AidexBroadcast, _ h: AidexHistory) -> Bool {
    b.timeOffset >= 60 && (b.status & 0x3F) == 0 && h.recordStatus == 0
        && h.glucoseMgDl > 0 && h.glucoseMgDl < 0x3FF
}

// App state machine (TransmitterModel.refreshSensorState): bit-expand status & calTemp.
private func describeSensorState(status: UInt8, calTemp: UInt8, timeOffset: UInt16) -> String {
    let s0 = status & 0x01, c0 = calTemp & 0x01
    var mal: [String] = []
    if status & 0x02 != 0 { mal.append("malfunc1") }
    if status & 0x04 != 0 { mal.append("sensorMalfunction") }
    if status & 0x08 != 0 { mal.append("malfunc8") }
    if status & 0x10 != 0 { mal.append("malfunc16") }
    if status & 0x20 != 0 { mal.append("generalFault") }
    let state: String
    if s0 == 1 && c0 == 1 { state = "NEW/USED-SENSOR (needs newSensor start)" }
    else if s0 == 1 && c0 == 0 { state = "SENSOR-EXPIRED" }
    else if timeOffset < 60 { state = "WARMING-UP" }
    else { state = "ACTIVE" }
    return "sensorState=[\(state)\(mal.isEmpty ? "" : " " + mal.joined(separator: ","))]"
}

// MARK: - Standard Bluetooth SIG CGM characteristic decoding (diagnostics)

private func describeStandardCgmStatus(_ data: Data) -> String? {
    let b = [UInt8](data)
    guard b.count >= 5 else { return nil }
    let timeOffset = leU16(b, at: 0)
    let status = b[2], calTemp = b[3], warn = b[4]
    let statusBits = ["Session Stopped","Battery Low","Sensor Type Incorrect","Sensor Malfunction","Device Specific Alert","General Device Fault"]
    let calBits = ["Time Sync Required","Calibration Not Allowed","Calibration Recommended","Calibration Required","Sensor Temp High","Sensor Temp Low"]
    func flags(_ v: UInt8, _ names: [String]) -> String {
        let on = names.enumerated().filter { v & (1 << $0.offset) != 0 }.map { $0.element }
        return on.isEmpty ? "(none)" : on.joined(separator: ",")
    }
    return String(format: "STD_CGM_STATUS timeOffset=%d status=0x%02X[%@] calTemp=0x%02X[%@] warning=0x%02X",
                  timeOffset, status, flags(status, statusBits), calTemp, flags(calTemp, calBits), warn)
}

// MARK: - AES-128-CFB128

private func aes128CFB(_ data: Data, operation: CCOperation, key: Data, iv: Data) -> Data? {
    guard key.count == kCCKeySizeAES128, iv.count == kCCBlockSizeAES128 else { return nil }
    var cryptor: CCCryptorRef?
    let createStatus = key.withUnsafeBytes { keyBytes in
        iv.withUnsafeBytes { ivBytes in
            CCCryptorCreateWithMode(operation, CCMode(kCCModeCFB), CCAlgorithm(kCCAlgorithmAES),
                                    CCPadding(ccNoPadding), ivBytes.baseAddress, keyBytes.baseAddress,
                                    key.count, nil, 0, 0, 0, &cryptor)
        }
    }
    guard createStatus == kCCSuccess, let cryptor else { return nil }
    defer { CCCryptorRelease(cryptor) }
    var output = [UInt8](repeating: 0, count: data.count + kCCBlockSizeAES128)
    var moved = 0
    let updateStatus = data.withUnsafeBytes { inputBytes in
        CCCryptorUpdate(cryptor, inputBytes.baseAddress, data.count, &output, output.count, &moved)
    }
    guard updateStatus == kCCSuccess else { return nil }
    return Data(output.prefix(moved))
}

private func devComm2Encode(command: UInt8, payload: Data, key: Data, iv: Data) -> Data? {
    var plaintext = [command]
    plaintext.append(contentsOf: payload)
    let checksum = crc16CCITT(plaintext)
    plaintext.append(UInt8(checksum & 0xFF))
    plaintext.append(UInt8((checksum >> 8) & 0xFF))
    return aes128CFB(Data(plaintext), operation: CCOperation(kCCEncrypt), key: key, iv: iv)
}

private func devComm2Decode(_ encrypted: Data, key: Data, iv: Data) -> (command: UInt8, payload: Data)? {
    guard encrypted.count >= 3,
          let plaintext = aes128CFB(encrypted, operation: CCOperation(kCCDecrypt), key: key, iv: iv) else { return nil }
    let bytes = [UInt8](plaintext)
    let expected = UInt16(bytes[bytes.count - 2]) | (UInt16(bytes[bytes.count - 1]) << 8)
    let actual = crc16CCITT(Array(bytes.dropLast(2)))
    guard expected == actual else { return nil }
    return (bytes[0], Data(bytes[1..<(bytes.count - 2)]))
}

// MARK: - Serial / key derivation

private func aidexBase36(_ character: Character) -> UInt8? {
    guard let ascii = character.asciiValue else { return nil }
    switch ascii {
    case 48...57: return ascii - 48
    case 65...90: return ascii - 55
    case 97...122: return ascii - 87
    default: return nil
    }
}

private func aidexSecret(serial: String) -> Data? {
    let values = serial.compactMap(aidexBase36)
    guard values.count == serial.count else { return nil }
    return Data(Insecure.MD5.hash(data: Data(values.map { UInt8(truncatingIfNeeded: Int($0) * 13 + 61) })))
}

private func aidexIV(serial: String) -> Data? {
    let values = serial.compactMap(aidexBase36)
    guard values.count == serial.count else { return nil }
    return Data(Insecure.MD5.hash(data: Data(values.map { UInt8(truncatingIfNeeded: Int($0) * 17 + 19) })))
}

// 9-byte newSensor datetime payload: year(u16 LE), month, day, hour, minute, second,
// timeZone(quarter-hours from UTC, signed), dstOffset(quarter-hours, signed).
private func newSensorPayload(date: Date = Date()) -> Data {
    let tz = TimeZone.current
    var cal = Calendar(identifier: .gregorian)
    cal.timeZone = tz
    let c = cal.dateComponents([.year, .month, .day, .hour, .minute, .second], from: date)
    let year = UInt16(c.year ?? 2026)
    let total = tz.secondsFromGMT(for: date)
    let dstSec = Int(tz.daylightSavingTimeOffset(for: date))
    let rawSec = total - dstSec
    let tzField = Int8(truncatingIfNeeded: rawSec / 900)
    let dstField = Int8(truncatingIfNeeded: dstSec / 900)
    return Data([
        UInt8(year & 0xFF), UInt8((year >> 8) & 0xFF),
        UInt8(c.month ?? 1), UInt8(c.day ?? 1),
        UInt8(c.hour ?? 0), UInt8(c.minute ?? 0), UInt8(c.second ?? 0),
        UInt8(bitPattern: tzField), UInt8(bitPattern: dstField)
    ])
}

private func dataFromHex(_ value: String) -> Data? {
    let cleaned = value.filter { !$0.isWhitespace && $0 != ":" && $0 != "-" }
    guard cleaned.count.isMultiple(of: 2) else { return nil }
    var result = Data()
    var cursor = cleaned.startIndex
    while cursor < cleaned.endIndex {
        let next = cleaned.index(cursor, offsetBy: 2)
        guard let byte = UInt8(cleaned[cursor..<next], radix: 16) else { return nil }
        result.append(byte)
        cursor = next
    }
    return result
}

private func properties(_ value: CBCharacteristicProperties) -> [String] {
    var result: [String] = []
    if value.contains(.broadcast) { result.append("broadcast") }
    if value.contains(.read) { result.append("read") }
    if value.contains(.writeWithoutResponse) { result.append("writeWithoutResponse") }
    if value.contains(.write) { result.append("write") }
    if value.contains(.notify) { result.append("notify") }
    if value.contains(.indicate) { result.append("indicate") }
    if value.contains(.authenticatedSignedWrites) { result.append("signedWrite") }
    if value.contains(.extendedProperties) { result.append("extended") }
    return result
}

// MARK: - Options

private struct Options {
    var seconds: TimeInterval = 30
    var target: String?
    var connect = false        // generic discovery dump
    var aidex = false          // proprietary handshake
    var serial = ""            // provide via --serial (no device secret hardcoded)
    var aidexKey: Data?        // device-issued pair key (16 bytes hex) -> reconnect path
    var deviceInfo = false     // queue 0x10
    var startTime = false      // queue 0x21
    var startSensor = false    // queue 0x20 (IRREVERSIBLE: starts/commits the sensor)
    var confirmStart = false   // required second factor (--yes) for --start-sensor
    var autoUpdate = false     // queue 0x34 + 0x35 to stream

    init() {
        var args = Array(CommandLine.arguments.dropFirst())
        while !args.isEmpty {
            let arg = args.removeFirst()
            func next() -> String? { args.isEmpty ? nil : args.removeFirst() }
            switch arg {
            case "--seconds": if let v = next(), let d = Double(v) { seconds = d }
            case "--target": target = next()
            case "--connect": connect = true
            case "--aidex", "--aidex-pair": aidex = true
            case "--serial": if let v = next() { serial = v }
            case "--aidex-key": if let v = next() { aidexKey = dataFromHex(v) }
            case "--device-info": aidex = true; deviceInfo = true
            case "--start-time": aidex = true; startTime = true
            case "--start-sensor": aidex = true; startSensor = true
            case "--yes": confirmStart = true
            case "--auto-update": aidex = true; autoUpdate = true
            default: break
            }
        }
    }
}

// MARK: - Probe

private final class Probe: NSObject, CBCentralManagerDelegate, CBPeripheralDelegate {
    private let options: Options
    private var central: CBCentralManager!
    private var seen: Set<UUID> = []
    private var connectedPeripheral: CBPeripheral?
    private var timer: Timer?
    private var connectedAt: Date?

    // aidex state
    private var mainChar: CBCharacteristic?      // F002
    private var privateChar: CBCharacteristic?   // F001 (unpaired) or F003 (paired)
    private var mainSubscribed = false
    private var privateSubscribed = false
    private var pairKey: Data?
    private var sessionKey: Data?
    private var sessionIV: Data?
    private var secretSent = false
    private var handshakeReadTriggered = false
    private var commandQueue: [(cmd: UInt8, payload: Data, label: String)] = []
    private var commandIndex = 0
    private var queueStarted = false

    private var lastAdvDescription: [UUID: String] = [:]

    init(options: Options) {
        self.options = options
        self.pairKey = options.aidexKey
        super.init()
        central = CBCentralManager(delegate: self, queue: nil)
    }

    func centralManagerDidUpdateState(_ central: CBCentralManager) {
        let state: String
        switch central.state {
        case .unknown: state = "unknown"
        case .resetting: state = "resetting"
        case .unsupported: state = "unsupported"
        case .unauthorized: state = "unauthorized"
        case .poweredOff: state = "poweredOff"
        case .poweredOn: state = "poweredOn"
        @unknown default: state = "futureState"
        }
        print("CENTRAL state=\(state)")
        fflush(stdout)
        guard central.state == .poweredOn else {
            if [.unsupported, .unauthorized, .poweredOff].contains(central.state) { scheduleExit(after: 1) }
            return
        }
        print("SCAN seconds=\(Int(options.seconds)) duplicates=true")
        central.scanForPeripherals(withServices: nil, options: [CBCentralManagerScanOptionAllowDuplicatesKey: true])
        timer = Timer.scheduledTimer(withTimeInterval: options.seconds, repeats: false) { [weak self] _ in self?.finish() }
        if let timer { RunLoop.main.add(timer, forMode: .common) }
    }

    func centralManager(_ central: CBCentralManager, didDiscover peripheral: CBPeripheral,
                        advertisementData: [String: Any], rssi RSSI: NSNumber) {
        let isNew = seen.insert(peripheral.identifier).inserted
        let localName = advertisementData[CBAdvertisementDataLocalNameKey] as? String
        let manufacturer = advertisementData[CBAdvertisementDataManufacturerDataKey] as? Data
        let services = (advertisementData[CBAdvertisementDataServiceUUIDsKey] as? [CBUUID])?.map(\.uuidString).sorted().joined(separator: ",") ?? "-"

        if isNew || manufacturer != nil {
            print("ADV new=\(isNew) id=\(peripheral.identifier.uuidString) name=\(peripheral.name ?? "-")" +
                  " localName=\(localName ?? "-") rssi=\(RSSI) services=\(services)" +
                  " manufacturer=\(hex(manufacturer).isEmpty ? "-" : hex(manufacturer))")
        }
        if let manufacturer, let adv = decodeAidexAdvertisement(manufacturer) {
            let desc = describeBroadcast(adv.broadcast, crcValid: adv.crcValid,
                                         nativePaired: adv.nativePaired, aesInitialized: adv.aesInitialized)
            if lastAdvDescription[peripheral.identifier] != desc {
                lastAdvDescription[peripheral.identifier] = desc
                print("AIDEX_ADV id=\(peripheral.identifier.uuidString) \(desc)")
                for h in adv.broadcast.histories where glucoseIsValid(adv.broadcast, h) {
                    print(String(format: "AIDEX_GLUCOSE_VALID mgdl=%d mmol=%.1f timeOffset=%d quality=%d",
                                 h.glucoseMgDl, Double(h.glucoseMgDl) / 18.0, h.timeOffset, h.quality))
                }
            }
        }
        fflush(stdout)

        guard (options.connect || options.aidex), connectedPeripheral == nil,
              matches(peripheral, localName: localName, advertisementData: advertisementData) else { return }
        connectedPeripheral = peripheral
        peripheral.delegate = self
        central.stopScan()
        print("CONNECT id=\(peripheral.identifier.uuidString) name=\(peripheral.name ?? localName ?? "-")")
        fflush(stdout)
        central.connect(peripheral, options: nil)
    }

    private func matches(_ peripheral: CBPeripheral, localName: String?, advertisementData: [String: Any]) -> Bool {
        guard let target = options.target?.lowercased(), !target.isEmpty else { return true }
        let fields = [peripheral.identifier.uuidString, peripheral.name ?? "", localName ?? "",
                      hex(advertisementData[CBAdvertisementDataManufacturerDataKey] as? Data)]
        return fields.contains { $0.lowercased().contains(target) }
    }

    func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
        connectedAt = Date()
        print("CONNECTED id=\(peripheral.identifier.uuidString)")
        fflush(stdout)
        peripheral.discoverServices(nil)
    }

    func centralManager(_ central: CBCentralManager, didFailToConnect peripheral: CBPeripheral, error: Error?) {
        print("CONNECT_FAILED error=\(error?.localizedDescription ?? "-")")
        scheduleExit(after: 1)
    }

    func centralManager(_ central: CBCentralManager, didDisconnectPeripheral peripheral: CBPeripheral, error: Error?) {
        print("DISCONNECTED error=\(error?.localizedDescription ?? "-")")
        scheduleExit(after: 1)
    }

    func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
        if let error { print("SERVICES_ERROR \(error.localizedDescription)"); return }
        for service in peripheral.services ?? [] {
            print("SERVICE uuid=\(service.uuid.uuidString) primary=\(service.isPrimary)")
            peripheral.discoverCharacteristics(nil, for: service)
        }
        fflush(stdout)
    }

    func peripheral(_ peripheral: CBPeripheral, didDiscoverCharacteristicsFor service: CBService, error: Error?) {
        if let error { print("CHARACTERISTICS_ERROR \(error.localizedDescription)"); return }
        for ch in service.characteristics ?? [] {
            print("CHAR service=\(service.uuid.uuidString) uuid=\(ch.uuid.uuidString) properties=\(properties(ch.properties).joined(separator: ","))")
            if options.aidex {
                if ch.uuid == CBUUID(string: "F002") { mainChar = ch }
                else if ch.uuid == CBUUID(string: pairKey == nil ? "F001" : "F003") { privateChar = ch }
            } else if options.connect {
                if ch.properties.contains(.read) { peripheral.readValue(for: ch) }
                if ch.properties.contains(.notify) || ch.properties.contains(.indicate) {
                    peripheral.setNotifyValue(true, for: ch)
                }
            }
        }
        if options.aidex, service.uuid == CBUUID(string: "181F"), let p = privateChar, !privateSubscribed {
            print("AIDEX_SUBSCRIBE stage=private uuid=\(p.uuid.uuidString)")
            peripheral.setNotifyValue(true, for: p)
        }
        fflush(stdout)
    }

    func peripheral(_ peripheral: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic, error: Error?) {
        let uuid = characteristic.uuid.uuidString
        let value = characteristic.value
        print("VALUE char=\(uuid) hex=\(hex(value)) error=\(error?.localizedDescription ?? "-")")

        // Generic discovery: decode standard CGM + Device Information.
        if options.connect, let v = value {
            switch uuid {
            case "2AA9": if let s = describeStandardCgmStatus(v) { print(s) }
            case "2A29", "2A24", "2A25", "2A28":
                let str = String(bytes: v.filter { $0 != 0 }, encoding: .utf8) ?? ""
                print("DEVINFO \(uuid)=\"\(str)\"")
            default: break
            }
        }

        guard options.aidex, let v = value else { fflush(stdout); return }

        if uuid == "F001" {
            if secretSent, v.count == 16, pairKey == nil {
                pairKey = v
                print("AIDEX_PAIR_SUCCESS key=\(hex(v))")
                triggerHandshakeRead(peripheral)
            }
        } else if uuid == "F003" {
            handleReconnectChallenge(v, peripheral: peripheral)
        } else if uuid == "F002" {
            if sessionKey == nil, v.count == 17 {
                handleReconnectChallenge(v, peripheral: peripheral)
            } else if let key = sessionKey, let iv = sessionIV, let msg = devComm2Decode(v, key: key, iv: iv) {
                print(String(format: "AIDEX_MESSAGE command=0x%02X payload=%@", msg.command, hex(msg.payload)))
                describeMessage(command: msg.command, payload: msg.payload)
            } else {
                print("AIDEX_RX_UNDECODED char=F002 hex=\(hex(v))")
            }
        }
        fflush(stdout)
    }

    func peripheral(_ peripheral: CBPeripheral, didUpdateNotificationStateFor characteristic: CBCharacteristic, error: Error?) {
        print("NOTIFY_STATE char=\(characteristic.uuid.uuidString) active=\(characteristic.isNotifying) error=\(error?.localizedDescription ?? "-")")
        guard options.aidex else { fflush(stdout); return }
        let uuid = characteristic.uuid.uuidString
        if uuid == "F001" || uuid == "F003" {
            privateSubscribed = characteristic.isNotifying
            if privateSubscribed, let m = mainChar, !mainSubscribed {
                print("AIDEX_SUBSCRIBE stage=main uuid=F002")
                peripheral.setNotifyValue(true, for: m)
            }
        } else if uuid == "F002" {
            mainSubscribed = characteristic.isNotifying
            if mainSubscribed && pairKey == nil && !secretSent {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { [weak self, weak peripheral] in
                    guard let self, let peripheral else { return }
                    self.sendPairingSecret(peripheral)
                }
            } else if mainSubscribed && sessionKey == nil && pairKey != nil {
                triggerHandshakeRead(peripheral)
            } else if mainSubscribed && sessionKey != nil {
                startCommandQueue(peripheral)
            }
        }
        fflush(stdout)
    }

    func peripheral(_ peripheral: CBPeripheral, didWriteValueFor characteristic: CBCharacteristic, error: Error?) {
        print("WRITE_RESULT char=\(characteristic.uuid.uuidString) error=\(error?.localizedDescription ?? "-")")
        fflush(stdout)
    }

    // MARK: handshake

    private func sendPairingSecret(_ peripheral: CBPeripheral) {
        guard !secretSent, let p = privateChar, let secret = aidexSecret(serial: options.serial) else {
            print("AIDEX_PAIR_ERROR invalid serial or missing F001"); return
        }
        secretSent = true
        print("AIDEX_SECRET serial=\(options.serial) char=F001 hex=\(hex(secret))")
        let type: CBCharacteristicWriteType = p.properties.contains(.writeWithoutResponse) ? .withoutResponse : .withResponse
        peripheral.writeValue(secret, for: p, type: type)
    }

    private func triggerHandshakeRead(_ peripheral: CBPeripheral) {
        guard !handshakeReadTriggered, let m = mainChar, m.properties.contains(.read) else { return }
        handshakeReadTriggered = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
            print("AIDEX_READ stage=sessionChallenge uuid=F002")
            peripheral.readValue(for: m)
            fflush(stdout)
        }
    }

    private func handleReconnectChallenge(_ encrypted: Data, peripheral: CBPeripheral) {
        guard sessionKey == nil, encrypted.count == 17, let key = pairKey, key.count == 16,
              let iv = aidexIV(serial: options.serial),
              let plaintext = aes128CFB(encrypted, operation: CCOperation(kCCDecrypt), key: key, iv: iv) else {
            print("AIDEX_RECONNECT_ERROR invalid challenge/key/serial"); return
        }
        let bytes = [UInt8](plaintext)
        let sk = Data(bytes.prefix(16))
        let valid = crc8Maxim(Array(bytes.prefix(16))) == bytes[16]
        print(String(format: "AIDEX_RECONNECT_CHALLENGE decrypted=%@ crc8valid=%@", hex(plaintext), valid ? "true" : "false"))
        guard valid else { return }
        sessionKey = sk
        sessionIV = iv
        print("AIDEX_SESSION_READY key=\(hex(sk)) iv=\(hex(iv))")
        if let m = mainChar, !mainSubscribed {
            print("AIDEX_SUBSCRIBE stage=main uuid=F002")
            peripheral.setNotifyValue(true, for: m)
        } else if mainSubscribed {
            startCommandQueue(peripheral)
        }
    }

    // MARK: command queue

    private func startCommandQueue(_ peripheral: CBPeripheral) {
        guard !queueStarted, sessionKey != nil, mainSubscribed else { return }
        queueStarted = true
        var q: [(UInt8, Data, String)] = []
        if options.deviceInfo { q.append((0x10, Data(), "getDeviceInfo")) }
        if options.startSensor {
            let payload = newSensorPayload()
            print("AIDEX_NEW_SENSOR_DATETIME payload=\(hex(payload)) (IRREVERSIBLE: starts a sensor session)")
            q.append((0x20, payload, "newSensor(START)"))
        }
        if options.startSensor || options.startTime { q.append((0x21, Data(), "getStartTime")) }
        if options.autoUpdate {
            q.append((0x34, Data([0x01]), "setAutoUpdate"))
            q.append((0x35, Data([0x01]), "setDynamicAdv"))
        }
        q.append((0x11, Data(), "getBroadcast"))   // always read current glucose last
        commandQueue = q
        commandIndex = 0
        sendNextCommand(peripheral)
    }

    private func sendNextCommand(_ peripheral: CBPeripheral) {
        guard commandIndex < commandQueue.count else {
            print("AIDEX_QUEUE_COMPLETE count=\(commandQueue.count)")
            fflush(stdout); return
        }
        let item = commandQueue[commandIndex]
        commandIndex += 1
        guard let key = sessionKey, let iv = sessionIV,
              let packet = devComm2Encode(command: item.cmd, payload: item.payload, key: key, iv: iv),
              let m = mainChar else {
            print("AIDEX_COMMAND_ERROR label=\(item.label)"); return
        }
        print(String(format: "AIDEX_COMMAND label=%@ command=0x%02X payload=%@ encrypted=%@",
                     item.label, item.cmd, hex(item.payload), hex(packet)))
        peripheral.writeValue(packet, for: m, type: .withoutResponse)
        fflush(stdout)
        let delay = item.cmd == 0x20 ? 2.0 : 1.0   // give the device time after a state change
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self, weak peripheral] in
            guard let self, let peripheral else { return }
            self.sendNextCommand(peripheral)
        }
    }

    private func reportValidGlucose(_ b: AidexBroadcast) {
        for h in b.histories where glucoseIsValid(b, h) {
            print(String(format: "AIDEX_GLUCOSE_VALID mgdl=%d mmol=%.1f timeOffset=%d quality=%d",
                         h.glucoseMgDl, Double(h.glucoseMgDl) / 18.0, h.timeOffset, h.quality))
        }
    }

    private func describeMessage(command: UInt8, payload: Data) {
        let bytes = [UInt8](payload)
        switch command {
        case 0x11:   // broadcast: [result][broadcast body...]
            guard let result = bytes.first else { return }
            print(String(format: "AIDEX_BROADCAST_RESULT status=0x%02X", result))
            if result == 1, let b = decodeBroadcastBody(Array(bytes.dropFirst())) {
                print("AIDEX_BROADCAST \(describeBroadcast(b))")
                reportValidGlucose(b)
            }
        case 0x20:
            print("AIDEX_NEW_SENSOR_ACK payload=\(hex(payload))")
        case 0x21:
            print("AIDEX_START_TIME payload=\(hex(payload))")
        case 0x10:
            print("AIDEX_DEVICE_INFO payload=\(hex(payload))")
        default:
            print("AIDEX_MESSAGE_OTHER command=0x\(String(format: "%02X", command)) payload=\(hex(payload))")
        }
    }

    private func finish() {
        if let peripheral = connectedPeripheral {
            let elapsed = connectedAt.map { Date().timeIntervalSince($0) } ?? 0
            print("FINISH connectedSeconds=\(String(format: "%.1f", elapsed))")
            central.cancelPeripheralConnection(peripheral)
            scheduleExit(after: 2)
        } else {
            central.stopScan()
            print("FINISH devices=\(seen.count)")
            scheduleExit(after: 0.2)
        }
        fflush(stdout)
    }

    private func scheduleExit(after seconds: TimeInterval) {
        DispatchQueue.main.asyncAfter(deadline: .now() + seconds) { exit(0) }
    }
}

private let options = Options()
if options.startSensor && !options.confirmStart {
    FileHandle.standardError.write(Data((
        "Refusing: --start-sensor is IRREVERSIBLE (commits the single-use sensor and begins a\n" +
        "~60-minute warmup). Re-run with --yes once a fresh sensor is applied.\n").utf8))
    exit(2)
}
if options.aidex && options.serial.isEmpty {
    FileHandle.standardError.write(Data("set --serial <device serial> for AiDEX operations\n".utf8))
    exit(2)
}
private let probe = Probe(options: options)
RunLoop.main.run()
