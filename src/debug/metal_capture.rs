use bevy::{
    prelude::*,
    render::{
        Render, RenderApp, RenderSystems, extract_resource::ExtractResource,
        extract_resource::ExtractResourcePlugin, renderer::RenderDevice,
    },
};
use objc2::runtime::AnyObject;
use objc2_foundation::{NSString, NSURL};
use objc2_metal::{MTLCaptureDescriptor, MTLCaptureDestination, MTLCaptureManager};
use std::{env::current_dir, time::SystemTime};

pub struct MetalCapturePlugin;

impl Plugin for MetalCapturePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FrameCapture>()
            .add_plugins(ExtractResourcePlugin::<FrameCapture>::default())
            .add_systems(Update, input_capture);

        app.sub_app_mut(RenderApp)
            .add_systems(Render, start_capture.in_set(RenderSystems::Prepare))
            .add_systems(Render, stop_capture.in_set(RenderSystems::Cleanup));
    }
}

#[derive(Clone, Default, Resource, ExtractResource)]
pub struct FrameCapture {
    pub(crate) capture: bool,
}

pub fn input_capture(input: Res<ButtonInput<KeyCode>>, mut capture: ResMut<FrameCapture>) {
    capture.capture = input.just_pressed(KeyCode::KeyC);
}

pub fn start_capture(capture: Res<FrameCapture>, device: Res<RenderDevice>) {
    if !capture.capture {
        return;
    }

    println!("Capturing frame");

    let output_path = current_dir().unwrap().join("captures").join(format!(
        "capture_{}.gputrace",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    ));

    let descriptor = MTLCaptureDescriptor::new();
    descriptor.setDestination(MTLCaptureDestination::GPUTraceDocument);
    descriptor.setOutputURL(Some(&NSURL::fileURLWithPath(&NSString::from_str(
        output_path.to_str().unwrap(),
    ))));

    unsafe {
        if let Some(device) = device.wgpu_device().as_hal::<wgpu_core::api::Metal>() {
            let raw_device: &AnyObject = AsRef::as_ref(&**device.raw_device());
            descriptor.setCaptureObject(Some(raw_device));
        }
    }

    let manager = unsafe { MTLCaptureManager::sharedCaptureManager() };
    if manager
        .startCaptureWithDescriptor_error(&descriptor)
        .is_err()
    {
        println!("Failed to start capture");
    }
}

pub fn stop_capture(capture: Res<FrameCapture>) {
    if !capture.capture {
        return;
    }

    unsafe { MTLCaptureManager::sharedCaptureManager() }.stopCapture();
}
