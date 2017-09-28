use nalgebra::{self as na, Transform3, Vector3, Point3, Vector2, Point2, Isometry3, IsometryMatrix3, Quaternion, Translation3, Unit};
use webvr::*;
use context::EyeContext;
use fnv::FnvHashMap;
use gfx::{Rect};
use ::NativeRepr;

pub struct VrContext {
    vrsm: VRServiceManager,
    disp: VRDisplayPtr,
    pub near: f64,
    pub far: f64,
    layer: VRLayer,
    exit: bool,
    paused: bool,
}

fn size_from_data(data: &VRDisplayData) -> (u32, u32) {
    let w = data.left_eye_parameters.render_width + data.right_eye_parameters.render_width;
    let h = data.left_eye_parameters.render_height.max(data.right_eye_parameters.render_height);
    (w, h)
}

impl VrContext {
    pub fn init(mut vrsm: VRServiceManager) -> Option<VrContext> {
        let display = match vrsm.get_displays().get(0) {
            Some(d) => d.clone(),
            None => {
                error!("No VR display present");
                return None
            },
        };
        info!("VR Device: {}", display.borrow().data().display_name);
        Some(VrContext {
            vrsm: vrsm,
            disp: display,
            near: 0.1,
            far: 100.0,
            layer: Default::default(),
            exit: false,
            paused: false,
        })
    }

    pub fn new() -> Option<VrContext> {
        let mut vrsm = VRServiceManager::new();
        vrsm.register_defaults();
        VrContext::init(vrsm)
    }

    pub fn mock() -> Option<VrContext> {
        let mut vrsm = VRServiceManager::new();
        vrsm.register_mock();
        VrContext::init(vrsm)
    }

    pub fn set_texture(&mut self, texture_id: u32) {
        info!("Attaching texture {} to HMD", texture_id);
        self.layer.texture_id = texture_id;
    }

    pub fn start(&mut self) {
        info!("Starting HMD presentation");
        self.disp.borrow_mut().start_present(Some(VRFramebufferAttributes {
            multiview: false,
            depth: false,
            multisampling: false,
        }));
    }

    pub fn stop(&mut self) {
        info!("Stopping HMD presentation");
        self.disp.borrow_mut().stop_present();
    }

    pub fn retrieve_size(&mut self) -> (u32, u32) {
       size_from_data(&self.disp.borrow().data())
    }

    pub fn sync(&mut self) -> VrMoment {
        for event in self.vrsm.poll_events() {
            match event {
                VREvent::Display(VRDisplayEvent::Pause(_)) => self.paused = true,
                VREvent::Display(VRDisplayEvent::Resume(_)) => self.paused = false,
                VREvent::Display(VRDisplayEvent::Exit(_)) => self.exit = true,
                _ => (),
            }
        }

        let mut moment = VrMoment {
            cont: FnvHashMap::default(),
            hmd: None,
            primary: None,
            secondary: None,
            tertiary: None,
            layer: self.layer.clone(),
            stage: na::one(),
            exit: self.exit,
            paused: self.paused,
        };
        {
            let mut disp = self.disp.borrow_mut();
            disp.sync_poses();
            let data = disp.data();
            let state = disp.synced_frame_data(self.near, self.far);
            let (w, h) = size_from_data(&data);

            moment.stage = if let Some(ref stage) = data.stage_parameters {
                Transform3::advanced(stage.sitting_to_standing_transform)
                    .try_inverse().unwrap_or(Transform3::identity())
            } else {
                Transform3::identity()
            };

            let left_view = Transform3::advanced(state.left_view_matrix);
            let right_view = Transform3::advanced(state.right_view_matrix);
            let left_projection = Transform3::advanced(state.left_projection_matrix);
            let right_projection = Transform3::advanced(state.right_projection_matrix);

            if let (Some(pose), true) = (pose_transform(&state.pose), data.connected) {
                moment.hmd = Some(Hmd {
                    name: data.display_name.clone(),
                    size: (w, h),
                    pose: na::convert(pose),
                    left: EyeContext {
                        eye: left_view.try_inverse().unwrap() * Point3::origin(),
                        view: left_view,
                        proj: left_projection,
                        clip_offset: -0.5,
                        clip: Rect {
                            x: 0,
                            y: 0,
                            w: data.left_eye_parameters.render_width as u16,
                            h: h as u16,
                        },
                    },
                    right: EyeContext {
                        eye: right_view.try_inverse().unwrap() * Point3::origin(),
                        view: right_view,
                        proj: right_projection,
                        clip_offset: 0.5,
                        clip: Rect {
                            x: data.left_eye_parameters.render_width as u16,
                            y: 0,
                            w: data.right_eye_parameters.render_width as u16,
                            h: h as u16,
                        },
                    },
                });
            }
        }
        let gamepads =  self.vrsm.get_gamepads();
        {
            let mut gpiter = gamepads.iter().map(|gp| gp.borrow().id());
            moment.primary = gpiter.next();
            moment.secondary = gpiter.next();
            moment.tertiary = gpiter.next();
        }
        for gp in gamepads {
            let gp = gp.borrow();
            let data = gp.data();
            let state = gp.state();
            if let Some(pose) = pose_transform(&state.pose) {
                moment.cont.insert(gp.id(), Controller {
                    name: data.name.clone(),
                    pose: na::convert(pose),
                    axes: state.axes.clone(),
                    buttons: state.buttons.clone(),
                });
            }
        }
        moment
    }
}

#[derive(Copy, Clone, Debug)]
pub enum ControllerRef {
    Primary,
    Secondary,
    Tertiary,
    Indexed(u32),
}

impl ControllerRef {
    fn index(&self, moment: &VrMoment) -> Option<u32> {
        use self::ControllerRef::*;
        match *self {
            Primary => moment.primary,
            Secondary => moment.secondary,
            Tertiary => moment.tertiary,
            Indexed(i) => Some(i),
        }
    }

    /// Make this always reference the current controller
    /// (will not change when controller role changes).
    pub fn fixed(&self, moment: &VrMoment) -> ControllerRef {
        match self.index(moment) {
            Some(i) => ControllerRef::Indexed(i),
            None => *self,
        }
    }
}

pub fn primary() -> ControllerRef {
    ControllerRef::Primary
}

pub fn secondary() -> ControllerRef {
    ControllerRef::Secondary
}

pub fn tertiary() -> ControllerRef {
    ControllerRef::Tertiary
}

pub type ControllerButton = VRGamepadButton;

pub trait Trackable {
    fn pose(&self) -> IsometryMatrix3<f32>;

    fn x_dir(&self) -> Vector3<f32> { self.pose() * Vector3::x() }
    fn y_dir(&self) -> Vector3<f32> { self.pose() * Vector3::y() }
    fn z_dir(&self) -> Vector3<f32> { self.pose() * Vector3::z() }
    fn origin(&self) -> Point3<f32> { self.pose() * Point3::origin() }
    fn pointing(&self) -> Vector3<f32> { -self.z_dir() }
}

#[derive(Clone)]
pub struct Hmd {
    pub name: String,
    pub size: (u32, u32),
    pub pose: IsometryMatrix3<f32>,
    pub left: EyeContext,
    pub right: EyeContext,
}

impl Trackable for Hmd {
    fn pose(&self) -> IsometryMatrix3<f32> {
        self.pose
    }
}

#[derive(Clone, Debug)]
pub struct Controller {
    pub name: String,
    pub pose: IsometryMatrix3<f32>,
    pub axes: Vec<f64>,
    pub buttons: Vec<ControllerButton>,
}

impl Trackable for Controller {
    fn pose(&self) -> IsometryMatrix3<f32> {
        self.pose
    }
}

pub type ControllerIter<'a> = ::std::collections::hash_map::Values<'a, u32, Controller>;

pub struct VrMoment {
    cont: FnvHashMap<u32, Controller>,
    hmd: Option<Hmd>,
    primary: Option<u32>,
    secondary: Option<u32>,
    tertiary: Option<u32>,
    layer: VRLayer,
    pub stage: Transform3<f32>,
    pub exit: bool,
    pub paused: bool,
}

impl VrMoment {
    pub fn controller(&self, role: ControllerRef) -> Option<&Controller> {
        if let Some(ref i) = role.index(self) { self.cont.get(i) } else { None }
    }

    pub fn controllers<'a>(&'a self) -> ControllerIter<'a> {
        self.cont.values()
    }

    pub fn hmd(&self) -> Option<&Hmd> {
        self.hmd.as_ref()
    }

    pub fn submit(self, ctx: &mut VrContext) {
        let mut d = ctx.disp.borrow_mut();
        d.render_layer(&self.layer);
        d.submit_frame();
    }
}

fn pose_transform(ctr: &VRPose) -> Option<Isometry3<f32>> {
    let or = Unit::new_normalize(Quaternion::advanced(
        match ctr.orientation { Some(o) => o, None => return None }));
    let pos = Translation3::advanced(
        match ctr.position { Some(o) => o, None => return None });
    Some(Isometry3::from_parts(pos, or))
}

/// A structure for tracking the state of a vive controller
pub struct ViveController {
    /// Which controller is connected to this state object
    pub is: ControllerRef,
    pub connected: bool,
    pub pose: IsometryMatrix3<f32>,
    pub pose_delta: IsometryMatrix3<f32>,
    pub trigger: f64,
    pub pad: Point2<f64>,
    pub pad_delta: Vector2<f64>,
    pub pad_touched: bool,
    pub menu: bool,
    pub grip: bool,
}

impl Default for ViveController {
    fn default() -> Self {
        ViveController {
            is: primary(),
            connected: false,
            pose: na::one(),
            pose_delta: na::one(),
            trigger: 0.,
            pad: Point2::origin(),
            pad_delta: na::zero(),
            pad_touched: false,
            menu: false,
            grip: false,
        }
    }
}

impl ViveController {
    pub fn update(&mut self, mom: &VrMoment) -> Result<(), ()> {
        if let Some(cont) = mom.controller(self.is) {
            if cont.axes.len() != 3 || cont.buttons.len() != 2 { return Err(()) }

            self.connected = true;

            self.pose_delta = cont.pose * self.pose.inverse();
            self.pose = cont.pose;

            let (x, y) = (cont.axes[0], cont.axes[1]);
            if x != 0. || y != 0. {
                let pad = Point2::new(x, y);
                self.pad_delta = pad - self.pad;
                self.pad = pad;
                self.pad_touched = true;
            } else { 
                self.pad_touched = false;
            }

            self.trigger = cont.axes[2];
            self.menu = cont.buttons[0].pressed;
            self.grip = cont.buttons[1].pressed;
        } else {
            self.pad_touched = false;
            self.menu = false;
            self.grip = false;
            self.trigger = 0.;
            self.connected = false;
        }
        Ok(())
    }

    pub fn pad_theta(&self) -> f64 {
        self.pad[1].atan2(self.pad[0])
    }
}

impl Trackable for ViveController {
    fn pose(&self) -> IsometryMatrix3<f32> {
        self.pose
    }
}