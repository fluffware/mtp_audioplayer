use log::info;

pub fn init_logging() {
    tracing_subscriber::fmt::init();
}

pub fn starting() {
    info!("Server starting");
}

pub fn ready() {
    info!("Server ready");
}

pub fn exiting() {
    info!("Server exiting");
}
