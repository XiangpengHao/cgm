//! Shared Tailwind class tokens. Centralizing these keeps buttons, inputs, and
//! cards visually consistent (one height, one radius, one focus ring) instead
//! of each component re-declaring divergent strings.

/// Focus ring applied to every interactive control (keyboard accessibility).
pub const FOCUS: &str = "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500 focus-visible:ring-offset-2 dark:focus-visible:ring-offset-slate-950";

/// Primary call-to-action button.
pub const BTN_PRIMARY: &str = "inline-flex items-center justify-center h-9 px-4 rounded-lg bg-sky-600 hover:bg-sky-500 active:bg-sky-700 text-white text-sm font-semibold disabled:opacity-50 disabled:pointer-events-none transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500 focus-visible:ring-offset-2 dark:focus-visible:ring-offset-slate-950";

/// Secondary / neutral button (bordered).
pub const BTN_GHOST: &str = "inline-flex items-center justify-center h-9 px-3 rounded-lg border border-slate-300 dark:border-slate-700 text-sm font-medium text-slate-700 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 active:bg-slate-200 dark:active:bg-slate-700 disabled:opacity-50 disabled:pointer-events-none transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500 focus-visible:ring-offset-2 dark:focus-visible:ring-offset-slate-950";

/// Compact neutral button (dense rows: device list, diagnostics).
pub const BTN_GHOST_SM: &str = "inline-flex items-center justify-center h-8 px-2.5 rounded-lg border border-slate-300 dark:border-slate-700 text-sm font-medium text-slate-700 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 active:bg-slate-200 dark:active:bg-slate-700 disabled:opacity-40 disabled:pointer-events-none transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500 focus-visible:ring-offset-2 dark:focus-visible:ring-offset-slate-950";

/// Destructive button (delete / clear).
pub const BTN_DANGER: &str = "inline-flex items-center justify-center h-9 px-3 rounded-lg bg-rose-600 hover:bg-rose-500 active:bg-rose-700 text-white text-sm font-semibold disabled:opacity-50 disabled:pointer-events-none transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rose-500 focus-visible:ring-offset-2 dark:focus-visible:ring-offset-slate-950";

/// Text input.
pub const INPUT: &str = "w-full h-9 px-3 rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-900 text-sm text-slate-800 dark:text-slate-100 placeholder:text-slate-400 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500";

/// Card surface.
pub const CARD: &str = "rounded-2xl bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800";

/// Muted secondary text (meets AA on both themes).
pub const MUTED: &str = "text-slate-500 dark:text-slate-400";
