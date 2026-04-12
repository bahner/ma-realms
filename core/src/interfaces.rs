pub use ma_core::interfaces::{DidPublisher, IpfsPublisher};

pub trait AclRuntime {
    fn can_enter(&self, actor_id: &str, room_id: &str, did_root: &str) -> bool;
    fn summary(&self, room_id: &str) -> String;
}
