use crate::frame::{object::Object, vulkan::Vulkan};
use crate::predictor::Controller;
use pipewire::prelude::*;
use pipewire::properties;
use pipewire::registry::Registry;
use std::{cell::RefCell, rc::Rc, thread, time::Duration};
use wayland_client::{
    protocol::{wl_output::WlOutput, wl_registry::WlRegistry},
    Display, EventQueue, GlobalManager, Main,
};

use pipewire::stream::{Stream, StreamFlags};
use pipewire::{Context, Core, MainLoop};
use wayland_protocols::unstable::xdg_output::v1::client::zxdg_output_manager_v1::ZxdgOutputManagerV1;
use wayland_protocols::unstable::xdg_output::v1::client::zxdg_output_v1::Event::Description;

const DELAY_SUCCESS: Duration = Duration::from_millis(100);
const DELAY_FAILURE: Duration = Duration::from_millis(1000);

#[derive(Clone)]
pub struct Capturer {
    event_queue: Rc<RefCell<EventQueue>>,
    globals: GlobalManager,
    mainloop: MainLoop,
    context: Rc<Context<MainLoop>>,
    core: Core,
    pw_registry: Rc<Registry>,
    wl_registry: Main<WlRegistry>,
    vulkan: Rc<Vulkan>,
    xdg_output_manager: Main<ZxdgOutputManagerV1>,
}
impl super::Capturer for Capturer {
    fn run(&self, output_name: &str, controller: Controller) {
        let controller = Rc::new(RefCell::new(controller));
        let mut stream = Stream::<i32>::simple(
            &self.mainloop,
            "wluma",
            properties! {
                *pipewire::keys::MEDIA_TYPE => "Video",
                *pipewire::keys::MEDIA_CATEGORY => "Capture",
            *pipewire::keys::MEDIA_ROLE => "Camera",
            },
        )
        .state_changed(|old, new| {
            println!("State changed: {:?} -> {:?}", old, new);
        })
        .process(|_stream, _user_data| {
            println!("On frame");
        })
        .create()
        .expect("Error creating stream");
        stream.connect(
            pipewire::spa::Direction::Input,
            None,
            StreamFlags::INACTIVE,
            &mut [],
        );
        println!("Created Stream");
        // self.globals
        //     .list()
        //     .iter()
        //     .filter(|(_, interface, _)| interface == "wl_output")
        //     .for_each(|(id, _, _)| {
        //         let output = Rc::new(self.wl_registry.bind::<WlOutput>(1, *id));
        //         let capturer = Rc::new(self.clone());
        //         let controller = controller.clone();
        //         let desired_output = output_name.to_string();
        //         self.xdg_output_manager
        //             .get_xdg_output(&output)
        //             .quick_assign(move |_, event, _| match event {
        //                 Description { description } if description.contains(&desired_output) => {
        //                     log::debug!(
        //                         "Using output '{}' for config '{}'",
        //                         description,
        //                         desired_output,
        //                     );
        //                     capturer
        //                         .clone()
        //                         .capture_frame(controller.clone(), output.clone());
        //                 }
        //                 _ => {}
        //             });
        //     });

        loop {
            self.event_queue
                .borrow_mut()
                .dispatch(&mut (), |_, _, _| {})
                .expect("Error running wlroots capturer main loop");
        }
    }
}

impl Default for Capturer {
    fn default() -> Self {
        let display = Display::connect_to_env().unwrap();
        let mut event_queue = display.create_event_queue();
        let attached_display = display.attach(event_queue.token());
        let wl_registry = attached_display.get_registry();
        let globals = GlobalManager::new(&attached_display);
        let mainloop = MainLoop::new().expect("Failed to init mainloop");
        let context = Rc::new(Context::new(&mainloop).expect("Failed to init context."));
        let core = context.connect(None).expect("Failed to connect");
        let pw_registry = Rc::new(core.get_registry().expect("Failed to connect to registy"));

        event_queue.sync_roundtrip(&mut (), |_, _, _| {}).unwrap();

        let xdg_output_manager = globals
            .instantiate_exact::<ZxdgOutputManagerV1>(3)
            .expect("Unable to init xdg_output_manager");

        let vulkan = Rc::new(Vulkan::new().expect("Unable to initialize Vulkan"));

        Self {
            event_queue: Rc::new(RefCell::new(event_queue)),
            globals,
            mainloop,
            context,
            core,
            pw_registry,
            wl_registry,
            vulkan,
            xdg_output_manager,
        }
    }
}

impl Capturer {
    fn capture_frame(
        self: Rc<Self>,
        controller: Rc<RefCell<Controller>>,
        output: Rc<Main<WlOutput>>,
    ) {
        // let mut frame = Object::default();
        // self.dmabuf_manager
        //     .capture_output(0, &output)
        //     .quick_assign(move |data, event, _| match event {
        //         Event::Frame {
        //             width,
        //             height,
        //             num_objects,
        //             ..
        //         } => {
        //             frame.set_metadata(width, height, num_objects);
        //         }
        //
        //         Event::Object {
        //             index, fd, size, ..
        //         } => {
        //             frame.set_object(index, fd, size);
        //         }
        //
        //         Event::Ready { .. } => {
        //             let luma = self
        //                 .vulkan
        //                 .luma_percent(&frame)
        //                 .expect("Unable to compute luma percent");
        //
        //             controller.borrow_mut().adjust(luma);
        //
        //             data.destroy();
        //
        //             thread::sleep(DELAY_SUCCESS);
        //             self.clone().capture_frame(controller.clone(), output.clone());
        //         }
        //
        //         Event::Cancel { reason } => {
        //             data.destroy();
        //
        //             if reason == CancelReason::Permanent {
        //                 panic!("Frame was cancelled due to a permanent error. If you just disconnected screen, this is not implemented yet.");
        //             } else {
        //                 log::error!("Frame was cancelled due to a temporary error, will try again.");
        //                 thread::sleep(DELAY_FAILURE);
        //                 self.clone().capture_frame(controller.clone(), output.clone());
        //             }
        //         }
        //
        //         _ => unreachable!(),
        //     });
    }

    pub fn context(&self) -> &Context<MainLoop> {
        &self.context
    }
}
