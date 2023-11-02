use crate::actions::action::{Action, ActionFuture};
use crate::actions::tag_dispatcher::TagDispatcher;
use crate::volume_control::VolumeControl;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
enum TagOrConst<D>
where
    D: TagDispatcher + Send,
{
    Tag {
        tag_name: String,
        dispatcher: Arc<D>,
    },
    Const(f32),
}

pub struct SetVolumeAction<D>
where
    D: TagDispatcher + Send,
{
    value: TagOrConst<D>,
    control: Arc<Mutex<VolumeControl>>,
}
impl<D> SetVolumeAction<D>
where
    D: TagDispatcher + Send,
{
    pub fn new_const(control: Arc<Mutex<VolumeControl>>, value: f32) -> SetVolumeAction<D> {
        SetVolumeAction {
            control,
            value: TagOrConst::Const(value),
        }
    }
    pub fn new_tag(
        control: Arc<Mutex<VolumeControl>>,
        tag_name: String,
        dispatcher: Arc<D>,
    ) -> SetVolumeAction<D> {
        SetVolumeAction {
            control,
            value: TagOrConst::Tag {
                tag_name,
                dispatcher,
            },
        }
    }
}

impl<D> Action for SetVolumeAction<D>
where
    D: TagDispatcher + Send + Sync + 'static,
{
    fn run(&self) -> ActionFuture {
        let control = self.control.clone();
        match &self.value {
            TagOrConst::Const(volume) => {
                let volume = *volume;
                Box::pin(async move {
                    let control = control.lock().unwrap();
                    control.set_volume(volume)?;
                    Ok(())
                })
            }
            TagOrConst::Tag {
                tag_name,
                dispatcher,
            } => {
                let dispatcher = dispatcher.clone();
                let tag_name = tag_name.clone();
                Box::pin(async move {
                    if let Some(vstr) = dispatcher.get_value(&tag_name) {
                        if let Ok(volume) = str::parse(&vstr) {
                            let control = control.lock().unwrap();
                            control.set_volume(volume)?;
                        }
                    }
                    Ok(())
                })
            }
        }
    }
}
