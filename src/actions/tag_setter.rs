use crate::util::error::DynResultFuture;

pub type TagSetFuture = DynResultFuture<()>;

pub trait TagSetter {
    fn set_tag(&self, tag_name: &str, value: &str) -> TagSetFuture;
}
