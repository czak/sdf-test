#[macro_use]
extern crate glium;
extern crate cgmath;
extern crate image;
extern crate rand;
extern crate rayon;
extern crate rusttype;

pub mod sdf;
use cgmath::Point2;
use glium::index::PrimitiveType;
use glium::texture::ClientFormat;
use glium::{glutin, Surface};
use rand::prelude::*;
use rayon::prelude::*;
use sdf::geometry::{Curve, Line};
use sdf::shape::{SegmentPrimitive, Shape};
use sdf::texture::Texture;
use std::alloc::System;
use std::io::prelude::*;
use std::time::Instant;

#[global_allocator]
static GLOBAL: System = System;

fn main() {
    let resx = std::env::args().nth(1).unwrap().parse::<u32>().unwrap();
    let resy = std::env::args().nth(2).unwrap().parse::<u32>().unwrap();
    let scale = std::env::args().nth(3).unwrap().parse::<f32>().unwrap();
    let shade = std::env::args().nth(4).unwrap().parse::<f32>().unwrap();
    let tex_scale = std::env::args().nth(5).unwrap().parse::<f32>().unwrap();
    let path = std::env::args().nth(6).unwrap();

    let mut font = Vec::<u8>::new();
    std::fs::File::open(path)
        .unwrap()
        .read_to_end(&mut font)
        .unwrap();
    let font = rusttype::Font::from_bytes(font).unwrap();

    let possible_glyphs = (32..)
        .filter_map(|n| std::char::from_u32(n))
        .filter_map(|c| {
            font.glyph(c)
                .scaled(rusttype::Scale::uniform(scale))
                .shape()
        });

    let (mut texture, mut allocator) = Texture::new(resx, resy);
    let mut views = Vec::new();

    let gen_time = Instant::now();
    for shape in possible_glyphs {
        let mut primitives = Vec::new();
        for contour in shape {
            for segment in contour.segments {
                match segment {
                    rusttype::Segment::Line(line) => {
                        primitives.push(SegmentPrimitive::Line(Line {
                            p0: Point2::new(line.p[0].x, line.p[0].y),
                            p1: Point2::new(line.p[1].x, line.p[1].y),
                        }))
                    }
                    rusttype::Segment::Curve(curve) => {
                        primitives.push(SegmentPrimitive::Curve(Curve {
                            p0: Point2::new(curve.p[0].x, curve.p[0].y),
                            p1: Point2::new(curve.p[1].x, curve.p[1].y),
                            p2: Point2::new(curve.p[2].x, curve.p[2].y),
                        }))
                    }
                }
            }
            primitives.push(SegmentPrimitive::End);
        }

        if let Some(view) = Shape::new(primitives, &mut allocator, shade) {
            views.push(view);
        } else {
            break;
        }
    }

    println!(
        "{} texture views allocated: {:?}",
        views.len(),
        gen_time.elapsed()
    );

    {
        let render_time = Instant::now();
        let lock = texture.lock();
        views.par_iter_mut().for_each(|view| {
            view.render(&lock);
        });
        println!("Rendered: {:?}", render_time.elapsed());
    }

    // let save_time = Instant::now();
    // image::png::PNGEncoder::new(std::fs::File::create("demo.png").unwrap())
    //     .encode(
    //         texture.get_data(),
    //         texture.get_width(),
    //         texture.get_height(),
    //         image::ColorType::RGB(8),
    //     ).unwrap();
    // println!("Saved image: {:?}", save_time.elapsed());

    let mut events_loop = glutin::EventsLoop::new();
    let window = glutin::WindowBuilder::new();
    let context = glutin::ContextBuilder::new();
    let display = glium::Display::new(window, context, &events_loop).unwrap();

    let vertex_buffer = {
        #[derive(Copy, Clone)]
        struct Vertex {
            position: [f32; 2],
            coord: [f32; 2],
        }

        implement_vertex!(Vertex, position, coord);

        glium::VertexBuffer::new(
            &display,
            &[
                Vertex {
                    position: [-1.0 * tex_scale, -1.0 * tex_scale],
                    coord: [0.0, 1.0],
                },
                Vertex {
                    position: [1.0 * tex_scale, -1.0 * tex_scale],
                    coord: [1.0, 1.0],
                },
                Vertex {
                    position: [1.0 * tex_scale, 1.0 * tex_scale],
                    coord: [1.0, 0.0],
                },
                Vertex {
                    position: [-1.0 * tex_scale, 1.0 * tex_scale],
                    coord: [0.0, 0.0],
                },
            ],
        ).unwrap()
    };

    let index_buffer = glium::IndexBuffer::new(
        &display,
        PrimitiveType::TrianglesList,
        &[0u16, 1, 2, 2, 3, 0],
    ).unwrap();

    let image = glium::texture::RawImage2d {
        data: std::borrow::Cow::Borrowed(texture.get_data()),
        width: texture.get_width(),
        height: texture.get_height(),
        format: ClientFormat::U8U8U8,
    };

    let texture = glium::texture::Texture2d::with_mipmaps(
        &display,
        image,
        glium::texture::MipmapsOption::NoMipmap,
    ).unwrap();

    let program = program!(&display, 140 => {
        vertex: r#"
            #version 140
            
            in vec2 position;
            in vec2 coord;
            out vec2 vCoord;

            void main() {
                gl_Position = vec4(position, 0.0, 1.0);
                vCoord = coord;
            }
        "#,
        fragment: r#"
            #version 140

            in vec2 vCoord;
            out vec4 color;

            uniform sampler2D tex;
            uniform vec2 mouse;

            float median(float a, float b, float c) {
                return max(min(a,b), min(max(a,b),c));
            }

            void main() {
                vec4 s = texture(tex, vCoord);
                float d = median(s.r, s.g, s.b);
                color = vec4(smoothstep(0.5 + mouse.x, 0.5 - mouse.x, d) * vec3(1.0), 1.0);
                color = mix(color, s, mouse.y);
            }
        "#,
    }).unwrap();

    let mut pos_x = 0.0;
    let mut pos_y = 0.0;
    let mut res_x = 0.0;
    let mut res_y = 0.0;

    let draw = |mouse_x: f32, mouse_y: f32| {
        let mut target = display.draw();
        target.clear_color(0.0, 0.0, 0.0, 1.0);
        target
            .draw(
                &vertex_buffer,
                &index_buffer,
                &program,
                &uniform!{
                    tex: &texture,
                    mouse: [mouse_x, mouse_y]
                },
                &Default::default(),
            ).unwrap();
        target.finish().unwrap();
    };

    draw(pos_x / res_x, pos_y / res_y);

    events_loop.run_forever(|event| {
        match event {
            glutin::Event::WindowEvent { event, .. } => match event {
                glutin::WindowEvent::CloseRequested => return glutin::ControlFlow::Break,
                glutin::WindowEvent::CursorMoved { position, .. } => {
                    pos_x = position.x as f32;
                    pos_y = position.y as f32;
                    draw(pos_x / res_x, pos_y / res_y);
                }
                glutin::WindowEvent::Resized(position) => {
                    res_x = position.width as f32;
                    res_y = position.height as f32;
                    draw(pos_x / res_x, pos_y / res_y);
                }
                _ => (),
            },
            _ => (),
        }
        glutin::ControlFlow::Continue
    });
}