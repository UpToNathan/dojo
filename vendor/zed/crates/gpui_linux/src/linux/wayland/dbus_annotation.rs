//! Client bindings for the locally pinned draft xdg_dbus_annotation protocol.
pub(crate) mod client {
    pub use self::generated::client::*;

    mod generated {
        pub mod client {
            #![allow(dead_code, non_camel_case_types, unused_unsafe, unused_variables)]
            #![allow(non_upper_case_globals, non_snake_case, unused_imports)]
            #![allow(missing_docs, clippy::all)]

            use wayland_client;
            use wayland_client::protocol::*;

            pub mod __interfaces {
                use wayland_client;
                use wayland_client::protocol::__interfaces::*;
                wayland_scanner::generate_interfaces!("resources/xdg-dbus-annotation-v1.xml");
            }
            use self::__interfaces::*;
            wayland_scanner::generate_client_code!("resources/xdg-dbus-annotation-v1.xml");
        }
    }
}
