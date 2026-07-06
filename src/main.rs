use std::{ops::{Add, Sub}, sync::Arc};
use cosmic_text::skrifa::font;
use pollster::FutureExt;
use ropey::Rope;
use wgpu::{InstanceDescriptor, MultisampleState, wgt::TextureViewDescriptor};
use winit::{application::ApplicationHandler, event::WindowEvent, event_loop::EventLoop, keyboard::{Key, NamedKey::Space}, window::Window};

struct GPU {
    device:     wgpu::Device,
    queue:      wgpu::Queue,
    surface:    wgpu::Surface<'static>,
    config:     wgpu::SurfaceConfiguration,
}

struct FontEngine {
    font_system: glyphon::FontSystem,
    cache: glyphon::Cache,
    swash_cache: glyphon::SwashCache,
    text_renderer: glyphon::TextRenderer,
    atlas: glyphon::TextAtlas,
    viewport: glyphon::Viewport,  
}

struct Cursor{
    buffer: glyphon::Buffer,
    screen_position_x: f32,
    screen_position_y: f32,
    text_position: usize
}
#[derive(Default)]
struct App{
    window:         Option<Arc<Window>>,
    gpu:            Option<GPU>,
    text_buffer:    Option<glyphon::Buffer>,
    cursor_buffer:  Option<glyphon::Buffer>,
    cursor:         Option<Cursor>,
    font_engine:    Option<FontEngine>,
    text:           ropey::Rope,
    control:        bool,
    scroll_line_offset: usize,

}
impl App{

    fn create_gpu(window: Arc<Window>) -> GPU {
         //Create a GPU instance and configure it
            let instance = wgpu::Instance::new(InstanceDescriptor {
                backends: wgpu::Backends::all(),
                flags: wgpu::InstanceFlags::all(),
                memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
                backend_options: wgpu::BackendOptions::default(),
                display: None
            });


            //Set up a surface and adapter for the window
            let surface = instance.create_surface(window.clone()).unwrap();
            let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            }).block_on().unwrap();
            let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
                label: None,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::default(),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                trace: wgpu::Trace::Off
            }).block_on().unwrap();
            
            let window_size = window.inner_size();
            let config = surface.get_default_config(&adapter, window_size.width, window_size.height).unwrap();
            surface.configure(&device, &config);

            GPU {
                device,
                queue,
                surface,
                config,
            }
    }

    fn create_font_engine(gpu: &GPU) -> FontEngine {
        let mut db = glyphon::fontdb::Database::new();
        db.load_system_fonts();

        let font_data = include_bytes!("FiraCode.ttf");
        db.load_font_data(font_data.to_vec());

        db.set_monospace_family("Fira Code");
        let font_system = glyphon::FontSystem::new_with_locale_and_db("en-US".into(), db);
        let cache = glyphon::Cache::new(&gpu.device);
        let mut viewport = glyphon::Viewport::new(&gpu.device, &cache);
        viewport.update(&gpu.queue, glyphon::Resolution { width: gpu.config.width, height: gpu.config.height });

        let mut text_atlas = glyphon::TextAtlas::new(&gpu.device, &gpu.queue, &cache, gpu.config.format);

        let text_renderer = glyphon::TextRenderer::new(&mut text_atlas, &gpu.device, MultisampleState::default(), None);

        FontEngine {
            font_system,
            cache: cache,
            swash_cache: glyphon::SwashCache::new(),
            text_renderer,
            atlas: text_atlas,
            viewport: viewport,
        }
    }

    fn create_text_buffer(font_engine: &mut FontEngine, window: &Window, start_text: &str) -> glyphon::Buffer {
        let mut buffer = glyphon::Buffer::new(&mut font_engine.font_system, glyphon::Metrics { font_size: 16.0, line_height: 20.0 });
        let size = window.inner_size();
        buffer.set_size(&mut font_engine.font_system, Some(size.width as f32), Some(size.height as f32));
        buffer.set_text(&mut font_engine.font_system, start_text, &glyphon::Attrs::new().family(glyphon::Family::Monospace), glyphon::Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut font_engine.font_system, false);
        buffer
    }

    fn get_cursor_position_on_screen(&self) -> (f32, f32){
        let cursor = self.cursor.as_ref().unwrap();
        let current_line = self.text.char_to_line(cursor.text_position); 

        let cursor_byte_index = self.text.char_to_byte(cursor.text_position);
        let line_start_by_index = self.text.line_to_byte(current_line);


        let byte_index_in_line = cursor_byte_index - line_start_by_index;
        let mut cursor_x = 0.0;
        let mut cursor_y = current_line as f32 * 20.0;
        let mut found = false;

        let buffer = self.text_buffer.as_ref().unwrap();

        for run in buffer.layout_runs(){
            if run.line_i == current_line{
                cursor_y = run.line_top;

                for glyph in run.glyphs.iter(){
                    if glyph.start == byte_index_in_line{
                        cursor_x = glyph.x;
                        found = true;
                    }
                }

                if !found{
                    if let Some(last_glyph) = run.glyphs.last(){
                        if byte_index_in_line > last_glyph.start{
                            cursor_x = last_glyph.x + last_glyph.w;
                        }
                    }
                }
            }
        }

        (cursor_x, cursor_y)

    }

    fn set_text(&mut self, text:String){
        self.text.insert(0, &text);
        self.text_buffer.as_mut().unwrap().set_text(&mut self.font_engine.as_mut().unwrap().font_system, &text, &glyphon::Attrs::new().family(glyphon::Family::Monospace), glyphon::Shaping::Advanced, None);
    }

    fn calculate_ctrl_backspace_target(text: &ropey::Rope, current_pos: usize)->usize{
        if current_pos == 0{
            return 0
        }
        let mut chars = text.chars_at(current_pos);
        let mut new_pos = current_pos;

        let mut found_non_whitespace = false;
        let mut is_alphanumeric_word = false;

        while let Some(c) = chars.prev(){
            if !found_non_whitespace{
                if c.is_whitespace(){
                    new_pos -=1;
                    continue;
                }
                
                found_non_whitespace = true;
                is_alphanumeric_word = c.is_alphanumeric();
                new_pos -= 1;
                continue;

            }

            if is_alphanumeric_word{
                if c.is_alphanumeric(){
                    new_pos-=1;
                }
                else {
                    break;
                }
            }else {
                if !c.is_alphanumeric() && !c.is_whitespace(){
                    new_pos-=1;
                }
                else{
                    break;
                }
            }
        }
        new_pos
        
    }
}

impl ApplicationHandler for App {

    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {

        if self.window.is_none() {
            self.text = ropey::Rope::new();
            self.text.insert(0, &generate_big_text_file());
            let window = Arc::new(event_loop.create_window(Window::default_attributes()).unwrap());
            //Create a window that can be seen

            self.gpu = Some(Self::create_gpu(window.clone()));
            self.font_engine = Some(Self::create_font_engine(&self.gpu.as_ref().unwrap()));
            self.text_buffer = Some(Self::create_text_buffer(self.font_engine.as_mut().unwrap(), &window, &self.text.to_string()));
            self.window = Some(window);

            
            let cursor_position = self.text.len_chars();
            let mut cursor_buffer = glyphon::Buffer::new(&mut self.font_engine.as_mut().unwrap().font_system, glyphon::Metrics { font_size: 16.0, line_height: 20.0 });
            
            cursor_buffer.set_text(&mut self.font_engine.as_mut().unwrap().font_system, "|", &glyphon::Attrs::new().family(glyphon::Family::Monospace), glyphon::Shaping::Advanced, None);
            self.cursor = Some(Cursor { buffer: cursor_buffer, screen_position_x: 0.0, screen_position_y: 0.0, text_position: cursor_position });
        }
        

    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        match event {
            WindowEvent::RedrawRequested => {
                println!("Redraw requested for window {:?}", window_id);
                let surface_texture = match self.gpu.as_ref().unwrap().surface.get_current_texture(){
                    wgpu::CurrentSurfaceTexture::Success(surf_tex)=> surf_tex,
                    wgpu::CurrentSurfaceTexture::Suboptimal(surf_tex) =>{
                        self.gpu.as_ref().unwrap().surface.configure(&self.gpu.as_ref().unwrap().device, &self.gpu.as_ref().unwrap().config);
                        surf_tex
                    },
                    _ => {
                        println!("Failed to acquire next swap chain texture!");
                        return;
                    }
                };
                let view = surface_texture.texture.create_view(&TextureViewDescriptor::default());
                let mut encoder = self.gpu.as_ref().unwrap().device.create_command_encoder(&wgpu::CommandEncoderDescriptor{
                    label: Some("Redraw Encoder"),
                });
                {
                    let (pos_x,pos_y) = self.get_cursor_position_on_screen();
                    let gpu = &self.gpu.as_ref().unwrap();
                    let font_engine = self.font_engine.as_mut().unwrap();
                    let cursor= self.cursor.as_ref().unwrap();
                    font_engine.text_renderer.prepare(
                        &gpu.device, 
                        &gpu.queue,
                        &mut font_engine.font_system,
                        &mut font_engine.atlas,
                        &font_engine.viewport,
                        [
                            // Main text
                            glyphon::TextArea { 
                                buffer: self.text_buffer.as_ref().unwrap(), 
                                left: 10.0, 
                                top: 10.0, 
                                scale: 1.0,
                                bounds: glyphon::TextBounds { left: 0, top: 0, right: 60000, bottom: 60000 }, 
                                default_color: glyphon::Color::rgba(0, 0, 0, 255), 
                                custom_glyphs: &[]
                            },
                            // Cursor
                            glyphon::TextArea { 
                                buffer: &cursor.buffer, 
                                left: 10.0 + pos_x - 3.0, 
                                top: pos_y + 10.0, 
                                scale: 1.0,
                                bounds: glyphon::TextBounds { left: 0, top: 0, right: 60000, bottom: 60000 }, 
                                default_color: glyphon::Color::rgba(255, 0, 255, 255), 
                                custom_glyphs: &[]
                            },
                            
                        ],
                        &mut font_engine.swash_cache
                    ).unwrap();
                    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor{
                        label: Some("Redraw Render Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment{
                            view: &view,
                            resolve_target: None,
                            ops: wgpu::Operations{
                                load: wgpu::LoadOp::Clear(wgpu::Color{
                                    r: 0.9,
                                    g: 0.9,
                                    b: 0.9,
                                    a: 1.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });

                    // Let glyphon draw the quads using the render pass you just made!
                    font_engine.text_renderer.render(&font_engine.atlas, &font_engine.viewport,&mut render_pass).unwrap();
                }
                self.gpu.as_ref().unwrap().queue.submit(std::iter::once(encoder.finish()));
                surface_texture.present();
            },
            WindowEvent::CloseRequested => {
                println!("Close requested for window {:?}", window_id);
                event_loop.exit();
            },
            WindowEvent::Resized(size) =>{
                println!("Window {:?} resized to {:?}", window_id, size);
                let gpu = self.gpu.as_mut().unwrap();
                gpu.config.width = size.width;
                gpu.config.height = size.height;
                gpu.surface.configure(&gpu.device, &gpu.config);
                self.font_engine.as_mut().unwrap().viewport.update(&gpu.queue, glyphon::Resolution { width: size.width, height: size.height });

                // Update the text buffer's physical boundaries so it knows where to word-wrap
                if let Some(text_buffer) = self.text_buffer.as_mut() {
                    text_buffer.set_size(
                        &mut self.font_engine.as_mut().unwrap().font_system, 
                        Some(size.width as f32), 
                        Some(size.height as f32)
                    );
                    // Force the CPU to recalculate the line breaks based on the new size
                    text_buffer.shape_until_scroll(&mut self.font_engine.as_mut().unwrap().font_system, false);
                }
                self.window.as_ref().unwrap().request_redraw();
            },
            WindowEvent::KeyboardInput { event,.. } => {  
                let cursor = self.cursor.as_mut().unwrap();
                let mut text_changed = false;
                if event.state == winit::event::ElementState::Pressed {
                    match event.logical_key.as_ref(){
                        Key::Character(c) => {
                            self.text.insert(cursor.text_position, c);
 
                            cursor.text_position = cursor.text_position.add(1);
                            text_changed = true;
                        },
                        Key::Named(winit::keyboard::NamedKey::Space) =>{
                            let c = " ";
                            self.text.insert(cursor.text_position, c);
  
                            cursor.text_position = cursor.text_position.add(1);
                            text_changed = true;
                        }
                        Key::Named(winit::keyboard::NamedKey::Backspace) =>{
                            if cursor.text_position <= 0{return}
                            let target_pos = if self.control{
                                Self::calculate_ctrl_backspace_target(&self.text, cursor.text_position)
                            }else{
                                cursor.text_position.saturating_sub(1)
                            };

                            self.text.remove(target_pos..cursor.text_position);
                            cursor.text_position = target_pos;
                            text_changed = true;
                        }
                        Key::Named(winit::keyboard::NamedKey::Enter) => {
                            let c = "\n";
                            self.text.insert(cursor.text_position, c);
          
                            cursor.text_position = cursor.text_position.add(1);
                            text_changed = true;
                        }
                        Key::Named(winit::keyboard::NamedKey::ArrowLeft)=>{
                            if cursor.text_position == 0{
                                return;
                            }


                            let new_pos = cursor.text_position.sub(1);
                            cursor.text_position = new_pos;
                            self.window.as_ref().unwrap().request_redraw();
                        }
                        Key::Named(winit::keyboard::NamedKey::ArrowRight) =>{
                            let new_pos = cursor.text_position.add(1);
                            cursor.text_position = new_pos.clamp(cursor.text_position, self.text.len_chars()); 
                            self.window.as_ref().unwrap().request_redraw();
                        }
                        Key::Named(winit::keyboard::NamedKey::Control) =>{
                            self.control = true;
                        },
                        Key::Named(winit::keyboard::NamedKey::Tab) =>{
                             let c = "\t";
                            self.text.insert(cursor.text_position, c);
          
                            cursor.text_position = cursor.text_position.add(1);
                            text_changed = true;
                        }
                        _ =>{}
                    }
                    
                    if text_changed{
                            let text_buffer = self.text_buffer.as_mut().unwrap();
                            text_buffer.set_text(&mut self.font_engine.as_mut().unwrap().font_system, &self.text.to_string(), &glyphon::Attrs::new().family(glyphon::Family::Monospace), glyphon::Shaping::Advanced, None);
                            text_buffer.shape_until_scroll(&mut self.font_engine.as_mut().unwrap().font_system, false);
                            self.window.as_ref().unwrap().request_redraw();
                    }
  
                }else if event.state == winit::event::ElementState::Released{
                    match event.logical_key.as_ref(){
                        Key::Named(winit::keyboard::NamedKey::Control) => {
                            self.control = false;
                        },
                        _ => {}
                    } 
                }

                
            }
            _ => {}
        }
    }


}

fn generate_big_text_file() -> String{
    let mut text = String::new();
    text.push_str("Hello Fucker!");
    text
}

fn main() {
    let event_loop = EventLoop::new().unwrap();

    let mut app = App::default();

    let _ = event_loop.run_app(&mut app);
}
