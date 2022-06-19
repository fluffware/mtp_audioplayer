use crate::util::error::DynResult;
use alsa::mixer::{Mixer, Selem, SelemId};
use log::info;

pub struct VolumeControl {
    mixer: Mixer,
    selem_id: SelemId,
}

impl VolumeControl {
    pub fn new(device: &str) -> DynResult<VolumeControl> {
        let mixer = Mixer::new(device, false)?;
        let mut found_selem = None;
        for elem in mixer.iter() {
            if let Some(selem) = Selem::new(elem) {
                if selem.has_playback_volume() {
                    found_selem = Some(selem);
                    break;
                }
            }
        }
        let selem = if let Some(selem) = found_selem {
            selem
        } else {
            return Err(format!("No playback volume control found for {}", device).into());
        };
        let selem_id = selem.get_id();

        info!(
            "Using {} on {} as volume control",
            selem_id.get_name()?,
            device
        );

        Ok(VolumeControl { mixer, selem_id })
    }

    pub fn set_volume(&self, volume: f32) -> DynResult<()> {
        let selem = match self.mixer.find_selem(&self.selem_id) {
            Some(s) => s,
            None => return Err("Selem not found".into()),
        };
        let (min, max) = selem.get_playback_volume_range();
        selem
            .set_playback_volume_all(min + ((max - min) as f32 * volume.clamp(0.0, 1.0)) as i64)?;
        Ok(())
    }
}
