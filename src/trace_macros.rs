#[cfg(feature = "debug")]
#[macro_export]
macro_rules! trace_event {
    ($name:ident, $path:expr $(, $value:expr)* $(,)?) => {
        if $crate::trace::enabled() {
            $crate::trace::events::$name($path $(, $value)*);
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

#[cfg(feature = "debug")]
macro_rules! trace_context {
    ($path:expr) => {
        $crate::trace::file_context($path)
    };
}

#[cfg(not(feature = "debug"))]
macro_rules! trace_context {
    ($path:expr) => {
        ()
    };
}

#[cfg(feature = "debug")]
macro_rules! trace_value {
    ($value:expr) => {
        $value
    };
}
#[cfg(not(feature = "debug"))]
macro_rules! trace_value {
    ($value:expr) => {
        ()
    };
}
