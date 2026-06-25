//! Apple Health (HealthKit) writer. Saves the app's valid, downsampled readings
//! as Blood Glucose `HKQuantitySample`s.
//!
//! Implemented with `msg_send!` against the stable Objective-C selectors so it
//! is resilient to objc2 binding version drift. Requires the HealthKit
//! entitlement and `NSHealthUpdateUsageDescription` (see README.md).

use cgm_core::datetime::rfc3339_to_epoch_ms;
use cgm_core::health::HealthSample;
use futures::channel::oneshot;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2::{class, msg_send};
use objc2_foundation::{NSArray, NSDate, NSError, NSSet, NSString};

const BLOOD_GLUCOSE_ID: &str = "HKQuantityTypeIdentifierBloodGlucose";

/// Whether HealthKit is available on this device (false on iPad / Mac without it).
pub fn is_available() -> bool {
    unsafe {
        let cls = class!(HKHealthStore);
        let available: Bool = msg_send![cls, isHealthDataAvailable];
        available.as_bool()
    }
}

/// Request write authorization for Blood Glucose, then save the samples.
/// Returns the number written.
pub async fn write_samples(samples: Vec<HealthSample>) -> Result<usize, String> {
    if !is_available() {
        return Err("HealthKit is not available on this device".into());
    }
    if samples.is_empty() {
        return Ok(0);
    }

    unsafe {
        // HKHealthStore *store = [[HKHealthStore alloc] init];
        let store: Retained<AnyObject> = msg_send![class!(HKHealthStore), new];

        // HKQuantityType *type = [HKQuantityType quantityTypeForIdentifier:@"…BloodGlucose"];
        let id = NSString::from_str(BLOOD_GLUCOSE_ID);
        let qtype: Retained<AnyObject> =
            msg_send![class!(HKQuantityType), quantityTypeForIdentifier: &*id];

        // Authorization: requestAuthorizationToShareTypes:readTypes:completion:
        let share: Retained<NSSet<AnyObject>> = NSSet::from_retained_slice(&[qtype.clone()]);
        let (auth_tx, auth_rx) = oneshot::channel::<Result<(), String>>();
        let auth_tx = std::cell::RefCell::new(Some(auth_tx));
        let auth_block = block2::RcBlock::new(move |granted: Bool, err: *mut NSError| {
            let result = if granted.as_bool() {
                Ok(())
            } else if let Some(err) = err.as_ref() {
                Err(err.localizedDescription().to_string())
            } else {
                Err("Health authorization denied".into())
            };
            if let Some(tx) = auth_tx.borrow_mut().take() {
                let _ = tx.send(result);
            }
        });
        let _: () = msg_send![
            &*store,
            requestAuthorizationToShareTypes: &*share,
            readTypes: std::ptr::null::<NSSet<AnyObject>>(),
            completion: &*auth_block,
        ];
        auth_rx.await.map_err(|_| "authorization cancelled".to_string())??;

        // mg/dL unit.
        let unit_str = NSString::from_str("mg/dL");
        let unit: Retained<AnyObject> = msg_send![class!(HKUnit), unitFromString: &*unit_str];

        // Build the samples.
        let mut objs: Vec<Retained<AnyObject>> = Vec::with_capacity(samples.len());
        for s in &samples {
            let Some(ms) = rfc3339_to_epoch_ms(&s.time) else {
                continue;
            };
            let date: Retained<NSDate> =
                NSDate::dateWithTimeIntervalSince1970(ms as f64 / 1000.0);
            let quantity: Retained<AnyObject> = msg_send![
                class!(HKQuantity),
                quantityWithUnit: &*unit,
                doubleValue: s.glucose_mgdl as f64,
            ];
            let sample: Retained<AnyObject> = msg_send![
                class!(HKQuantitySample),
                quantitySampleWithType: &*qtype,
                quantity: &*quantity,
                startDate: &*date,
                endDate: &*date,
            ];
            objs.push(sample);
        }
        let count = objs.len();
        let array: Retained<NSArray<AnyObject>> = NSArray::from_retained_slice(&objs);

        // saveObjects:withCompletion:
        let (save_tx, save_rx) = oneshot::channel::<Result<(), String>>();
        let save_tx = std::cell::RefCell::new(Some(save_tx));
        let save_block = block2::RcBlock::new(move |ok: Bool, err: *mut NSError| {
            let result = if ok.as_bool() {
                Ok(())
            } else if let Some(err) = err.as_ref() {
                Err(err.localizedDescription().to_string())
            } else {
                Err("Health save failed".into())
            };
            if let Some(tx) = save_tx.borrow_mut().take() {
                let _ = tx.send(result);
            }
        });
        let _: () = msg_send![
            &*store,
            saveObjects: &*array,
            withCompletion: &*save_block,
        ];
        save_rx.await.map_err(|_| "save cancelled".to_string())??;
        Ok(count)
    }
}
