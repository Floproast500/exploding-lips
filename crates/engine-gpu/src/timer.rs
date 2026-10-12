//! GPU frame timing with timestamp queries (devices created with `TIMESTAMP_QUERY` and
//! `TIMESTAMP_QUERY_INSIDE_ENCODERS`): a timestamp before the world pass and after the last
//! post pass, read back a frame or more later without stalling.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) struct GpuTimer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    /// Nanoseconds per timestamp tick.
    period: f32,
    /// A readback is in flight (mapped or mapping): skip timing frames until it is read.
    pending: bool,
    /// The copy was recorded this frame and is not mapped yet.
    recorded: bool,
    mapped: Arc<AtomicBool>,
    last_ms: Option<f32>,
}

impl GpuTimer {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        let needed = wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
        if !device.features().contains(needed) {
            return None;
        }
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("frame time"), ty: wgpu::QueryType::Timestamp, count: 2 });
        let buffer = |label, usage| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: 16, usage, mapped_at_creation: false });
        Some(Self {
            queries,
            resolve: buffer("timestamps", wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC),
            readback: buffer("timestamps readback", wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ),
            period: queue.get_timestamp_period(),
            pending: false,
            recorded: false,
            mapped: Arc::new(AtomicBool::new(false)),
            last_ms: None,
        })
    }

    /// Whether this frame is timed (no readback in flight).
    pub(crate) fn begin(&mut self, encoder: &mut wgpu::CommandEncoder) -> bool {
        if self.pending {
            return false;
        }
        encoder.write_timestamp(&self.queries, 0);
        true
    }

    pub(crate) fn end(&mut self, encoder: &mut wgpu::CommandEncoder) {
        encoder.write_timestamp(&self.queries, 1);
        encoder.resolve_query_set(&self.queries, 0..2, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.readback, 0, 16);
        self.pending = true;
        self.recorded = true;
    }

    /// After the frame's commands are submitted: start mapping the readback.
    pub(crate) fn submitted(&mut self) {
        if std::mem::take(&mut self.recorded) {
            let mapped = self.mapped.clone();
            self.readback.slice(..).map_async(wgpu::MapMode::Read, move |r| mapped.store(r.is_ok(), Ordering::Release));
        }
    }

    /// The latest measurement (milliseconds), updated when a readback has completed.
    pub(crate) fn read(&mut self, device: &wgpu::Device) -> Option<f32> {
        let _ = device.poll(wgpu::PollType::Poll);
        if self.pending && self.mapped.swap(false, Ordering::Acquire) {
            {
                if let Ok(data) = self.readback.slice(..).get_mapped_range() {
                    let t: &[u64] = bytemuck::cast_slice(&data);
                    if t[1] > t[0] {
                        self.last_ms = Some((t[1] - t[0]) as f32 * self.period / 1.0e6);
                    }
                }
            }
            self.readback.unmap();
            self.pending = false;
        }
        self.last_ms
    }
}
