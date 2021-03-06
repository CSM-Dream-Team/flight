use gfx::{Resources, Encoder, Primitive, Rect, CommandBuffer, Slice, ShaderSet, Factory};
use gfx::handle::Buffer;
use gfx::traits::FactoryExt;
use gfx::state::Rasterizer;
use nalgebra::{Transform3};
use fnv::FnvHashMap;
use failure::Fail;
use std::cell::RefCell;

use ::{DepthRef, TargetRef, Error, FlightError, NativeRepr};
use ::mesh::{Mesh, Vertex};

#[macro_use]
mod shaders;
mod context;
pub use self::context::*;

mod solid;
pub use self::solid::{SolidStyle, SolidInputs};

mod unishade;
pub use self::unishade::{UnishadeStyle, UnishadeInputs};

mod pbr;
pub use self::pbr::{PbrStyle, PbrMaterial, PbrInputs, LIGHT_COUNT};

mod uber;
pub use self::uber::{UberStyle, UberMaterial, UberInputs, UberEnv};

/// The painter is responsible for drawing meshes. Painters
/// are instantiated with an associated style which specifies
/// the data required for drawing (vertex type, material params,
/// configuration) and implements the drawing pipeline. Note that
/// a painter can only be used with primitive types that have been
/// passed to `setup`.
pub struct Painter<R: Resources, E: Style<R>> {
    inputs: RefCell<E::Inputs>,
    map: FnvHashMap<Primitive, E>,
}

impl<R: Resources, E: Style<R>> Painter<R, E> {
    /// Create a new painter in the given style and using the given factory.
    pub fn new<F: Factory<R> + FactoryExt<R>>(f: &mut F) -> Result<Painter<R, E>, Error> {
        Ok(Painter {
            inputs: RefCell::new(E::init(f)?),
            map: Default::default(),
        })
    }

    /// Add the ability to draw the given primitive. This must be done before a mesh using
    /// the primitive is drawn.
    pub fn setup<F: Factory<R> + FactoryExt<R>>(&mut self, f: &mut F, prim: Primitive) -> Result<(), Error> {
        let mut inputs = self.inputs.borrow_mut();
        use ::std::collections::hash_map::Entry::*;
        match self.map.entry(prim) {
            Vacant(e) => {
                e.insert(E::new(f, &mut *inputs, prim, Rasterizer::new_fill())?);
            },
            _ => (),
        }
        Ok(())
    }

    /// Attempt to draw a mesh with the given parameters and model matrix,
    /// returning `Err` if something goes wrong.
    pub fn try_draw<C>(
        &self,
        ctx: &mut DrawParams<R, C>,
        model: Transform3<f32>,
        mesh: &Mesh<R, E::Vertex, E::Material>,
    )
        -> Result<(), Error>
        where C: CommandBuffer<R>
    {
        if let Some(ref sty) = self.map.get(&mesh.prim) {
            let mut inputs = self.inputs.borrow_mut();
            let mut trans = TransformBlock {
                eye: ctx.left.eye.to_homogeneous().downgrade(),
                model: model.downgrade(),
                view: ctx.left.view.downgrade(),
                proj: ctx.left.proj.downgrade(),
                clip_offset: ctx.left.clip_offset,
            };
            inputs.transform(trans.clone());
            sty.draw_raw(
                &mut *inputs,
                &mut ctx.encoder,
                ctx.color.clone(),
                ctx.depth.clone(),
                ctx.left.clip,
                &mesh.slice,
                mesh.buf.clone(),
                &mesh.mat,
            )?;

            trans.eye = ctx.right.eye.to_homogeneous().downgrade();
            trans.view = ctx.right.view.downgrade();
            trans.proj = ctx.right.proj.downgrade();
            trans.clip_offset = ctx.right.clip_offset;
            inputs.transform(trans);
            sty.draw_raw(
                &mut *inputs,
                &mut ctx.encoder,
                ctx.color.clone(),
                ctx.depth.clone(),
                ctx.right.clip,
                &mesh.slice,
                mesh.buf.clone(),
                &mesh.mat,
            )?;

            Ok(())
        } else {
            Err(
                FlightError::InvalidPrimitive { given: mesh.prim }
                .context("setup has not been done for this primitive type".to_owned())
                .into()
            )
        }
    }

    /// Draw a mesh with the given parameters and model matrix, logging any errors.
    pub fn draw<C>(
        &self,
        ctx: &mut DrawParams<R, C>,
        model: Transform3<f32>,
        mesh: &Mesh<R, E::Vertex, E::Material>,
    )
        where C: CommandBuffer<R>
    {
        if let Err(e) = self.try_draw(ctx, model, mesh) {
            error!("{}", e);
        }
    }

    /// Configure the draw style. For example, `cfg(|c| c.ambient([1., 0., 0., 1.]))`
    /// might set the ambient light color to red. The exact customization available
    /// depends on the style being used.
    pub fn cfg<F: FnOnce(&mut E::Inputs)>(&self, f: F) {
        f(&mut *self.inputs.borrow_mut())
    }
}

/// Implements a particular drawing process and visual style.
pub trait Style<R: Resources>: Sized {
    /// The mesh vertex type required for drawing
    type Vertex: Vertex;
    /// The configuration available for this style
    type Inputs: StyleInputs<R>;
    /// The material type required on meshes
    type Material;

    fn new<F: Factory<R> + FactoryExt<R>>(
        &mut F,
        &mut Self::Inputs,
        Primitive,
        Rasterizer,
    ) -> Result<Self, Error>;

    fn init<F: Factory<R> + FactoryExt<R>>(
        &mut F,
    ) -> Result<Self::Inputs, Error>;

    fn draw_raw<C>(
        &self,
        &mut Self::Inputs,
        &mut Encoder<R, C>,
        TargetRef<R>,
        DepthRef<R>,
        Rect,
        &Slice<R>,
        Buffer<R, Self::Vertex>,
        &Self::Material,
    )
        -> Result<(), Error>
        where C: CommandBuffer<R>;
}

/// Required configuration options for a `Style`
pub trait StyleInputs<R: Resources> {
    /// Transformation matrices and eye parameters
    fn transform(&mut self, block: TransformBlock);
    /// The set of shaders used by the styler
    fn shader_set(&self) -> &ShaderSet<R>;
}

mod defines {
    use ::{Light, NativeRepr};

    gfx_defines!{
        constant TransformBlock {
            model: [[f32; 4]; 4] = "model",
            view: [[f32; 4]; 4] = "view",
            proj: [[f32; 4]; 4] = "proj",
            eye: [f32; 4] = "eye_pos",
            clip_offset: f32 = "clip_offset",
        }
        constant LightBlock {
            pos: [f32; 4] = "pos",
            color: [f32; 4] = "color",
        }
    }

    impl From<Light> for LightBlock {
        fn from(l: Light) -> LightBlock {
            LightBlock {
                pos: l.pos.to_homogeneous().downgrade(),
                color: l.color,
            }
        }
    }
}
use self::defines::*;
