use crate::util::error::DynResult;
use log::info;

pub struct VolumeControl;

impl VolumeControl {
    pub fn new(_device: &str) -> DynResult<VolumeControl> {
        info!("Volume control not supported");
        Ok(VolumeControl)
    }

    pub fn set_volume(&self, _volume: f32) -> DynResult<()> {
        Ok(())
    }
}
