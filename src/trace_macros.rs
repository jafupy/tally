#[cfg(feature = "debug")]
#[macro_export]
macro_rules! trace_event {
    ($name:expr, $path:expr, $detail:expr) => {
        if $crate::trace::enabled() {
            $crate::trace::event($name, $path, $detail);
        }
    };
}

#[cfg(not(feature = "debug"))]
#[macro_export]
macro_rules! trace_event {
    ($($arg:tt)*) => {};
}

#[cfg(feature = "debug")]
#[macro_export]
macro_rules! trace_span {
    ($name:expr, $path:expr) => {
        $crate::trace::span($name, $path)
    };
}

#[cfg(not(feature = "debug"))]
#[macro_export]
macro_rules! trace_span {
    ($($arg:tt)*) => {
        ()
    };
}
