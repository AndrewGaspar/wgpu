use crate::{
    command::bind::{Binder, LayoutChange},
    hub::HUB,
    track::{Stitch, TrackerSet},
    BindGroupId,
    BufferAddress,
    CommandBuffer,
    CommandBufferId,
    ComputePassId,
    ComputePipelineId,
    BIND_BUFFER_ALIGNMENT,
    RawString,
    Stored,
};

use hal::{self, command::RawCommandBuffer};

use std::{iter, slice};

pub struct ComputePass<B: hal::Backend> {
    raw: B::CommandBuffer,
    cmb_id: Stored<CommandBufferId>,
    binder: Binder,
    trackers: TrackerSet,
}

impl<B: hal::Backend> ComputePass<B> {
    pub(crate) fn new(
        raw: B::CommandBuffer,
        cmb_id: Stored<CommandBufferId>,
        trackers: TrackerSet,
    ) -> Self {
        ComputePass {
            raw,
            cmb_id,
            binder: Binder::default(),
            trackers,
        }
    }
}

// Common routines between render/compute

#[no_mangle]
pub extern "C" fn wgpu_compute_pass_end_pass(pass_id: ComputePassId) -> CommandBufferId {
    let mut cmb_guard = HUB.command_buffers.write();
    let pass = HUB.compute_passes.unregister(pass_id);
    let cmb = &mut cmb_guard[pass.cmb_id.value];

    // There are no transitions to be made: we've already been inserting barriers
    // into the parent command buffer while recording this compute pass.
    cmb.trackers = pass.trackers;
    cmb.raw.push(pass.raw);
    pass.cmb_id.value
}

#[no_mangle]
pub extern "C" fn wgpu_compute_pass_set_bind_group(
    pass_id: ComputePassId,
    index: u32,
    bind_group_id: BindGroupId,
    offsets: *const BufferAddress,
    offsets_length: usize,
) {
    let mut pass_guard = HUB.compute_passes.write();
    let pass = &mut pass_guard[pass_id];
    let bind_group_guard = HUB.bind_groups.read();
    let bind_group = &bind_group_guard[bind_group_id];

    assert_eq!(bind_group.dynamic_count, offsets_length);
    let offsets = if offsets_length != 0 {
        unsafe { slice::from_raw_parts(offsets, offsets_length) }
    } else {
        &[]
    };

    if cfg!(debug_assertions) {
        for off in offsets {
            assert_eq!(
                *off % BIND_BUFFER_ALIGNMENT,
                0,
                "Misaligned dynamic buffer offset: {} does not align with {}",
                off,
                BIND_BUFFER_ALIGNMENT
            );
        }
    }

    //Note: currently, WebGPU compute passes have synchronization defined
    // at a dispatch granularity, so we insert the necessary barriers here.

    CommandBuffer::insert_barriers(
        &mut pass.raw,
        &mut pass.trackers,
        &bind_group.used,
        Stitch::Last,
        &*HUB.buffers.read(),
        &*HUB.textures.read(),
    );

    if let Some((pipeline_layout_id, follow_up)) =
        pass.binder
            .provide_entry(index as usize, bind_group_id, bind_group, offsets)
    {
        let pipeline_layout_guard = HUB.pipeline_layouts.read();
        let bind_groups =
            iter::once(bind_group.raw.raw()).chain(follow_up.map(|bg_id| bind_group_guard[bg_id].raw.raw()));
        unsafe {
            pass.raw.bind_compute_descriptor_sets(
                &pipeline_layout_guard[pipeline_layout_id].raw,
                index as usize,
                bind_groups,
                offsets.iter().map(|&off| off as hal::command::DescriptorSetOffset),
            );
        }
    };
}

#[no_mangle]
pub extern "C" fn wgpu_compute_pass_push_debug_group(_pass_id: ComputePassId, _label: RawString) {
    //TODO
}

#[no_mangle]
pub extern "C" fn wgpu_compute_pass_pop_debug_group(_pass_id: ComputePassId) {
    //TODO
}

#[no_mangle]
pub extern "C" fn wgpu_compute_pass_insert_debug_marker(
    _pass_id: ComputePassId,
    _label: RawString,
) {
    //TODO
}

// Compute-specific routines

#[no_mangle]
pub extern "C" fn wgpu_compute_pass_dispatch(pass_id: ComputePassId, x: u32, y: u32, z: u32) {
    unsafe {
        HUB.compute_passes.write()[pass_id].raw.dispatch([x, y, z]);
    }
}

#[no_mangle]
pub extern "C" fn wgpu_compute_pass_set_pipeline(
    pass_id: ComputePassId,
    pipeline_id: ComputePipelineId,
) {
    let mut pass_guard = HUB.compute_passes.write();
    let pass = &mut pass_guard[pass_id];
    let pipeline_guard = HUB.compute_pipelines.read();
    let pipeline = &pipeline_guard[pipeline_id];

    unsafe {
        pass.raw.bind_compute_pipeline(&pipeline.raw);
    }

    // Rebind resources
    if pass.binder.pipeline_layout_id != Some(pipeline.layout_id.clone()) {
        let pipeline_layout_guard = HUB.pipeline_layouts.read();
        let pipeline_layout = &pipeline_layout_guard[pipeline.layout_id];
        let bind_group_guard = HUB.bind_groups.read();

        pass.binder.pipeline_layout_id = Some(pipeline.layout_id.clone());
        pass.binder
            .reset_expectations(pipeline_layout.bind_group_layout_ids.len());

        for (index, (entry, &bgl_id)) in pass
            .binder
            .entries
            .iter_mut()
            .zip(&pipeline_layout.bind_group_layout_ids)
            .enumerate()
        {
            if let LayoutChange::Match(bg_id) = entry.expect_layout(bgl_id) {
                let desc_set = bind_group_guard[bg_id].raw.raw();
                unsafe {
                    pass.raw.bind_compute_descriptor_sets(
                        &pipeline_layout.raw,
                        index,
                        iter::once(desc_set),
                        &[],
                    );
                }
            }
        }
    }
}
