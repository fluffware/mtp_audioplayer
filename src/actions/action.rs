
pub trait Action
{
    fn run() -> Future<Output = Result<(). dyn Error + Send + Sync>>;
}
