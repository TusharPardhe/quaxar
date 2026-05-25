//! Generated protobuf message surface from `xrpl/protocol/messages.h`.

pub const TYPE_BOOL_MACRO_WORKAROUND: &str = "TYPE_BOOL";

include!(concat!(env!("OUT_DIR"), "/protocol.rs"));

#[cfg(test)]
mod tests {
    use super::{MessageType, TmPing};

    #[test]
    fn messages_module_exposes_generated_overlay_types() {
        let ping = TmPing::default();
        assert_eq!(ping.r#type, 0);
        assert_eq!(MessageType::MtPing as i32, 3);
    }
}
