use crate::util::error::DynResult;
use crate::util::error::DynResultFuture;

pub type TagSetFuture = DynResultFuture<()>;

pub trait TagSetter {
    fn async_set_tag(&self, tag_name: &str, value: &str) -> TagSetFuture;
    /* Does not guarantee that the tag is set when the function returns or at all. */
    fn set_tag(&self, tag_name: &str, value: &str) -> DynResult<()>;
}
