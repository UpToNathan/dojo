//! Cursor-shape requests resolve the controller's transient seat before Smithay dispatch.
use smithay::input::pointer::PointerHandle;
use smithay::reexports::wayland_protocols::wp::cursor_shape::v1::server::{
    wp_cursor_shape_device_v1::WpCursorShapeDeviceV1,
    wp_cursor_shape_manager_v1::{Request, WpCursorShapeManagerV1},
};
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource};
use smithay::wayland::{Dispatch2, GlobalData, GlobalDispatch2};

use crate::niri::State;

pub struct ManagerData;
pub struct RetiredPointer;

pub fn register(display: &DisplayHandle) {
    display.create_global::<State, WpCursorShapeManagerV1, _>(2, ManagerData);
}

impl GlobalDispatch2<WpCursorShapeManagerV1, State> for ManagerData {
    fn bind(
        &self,
        _state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<WpCursorShapeManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ManagerData);
    }
}

impl Dispatch2<WpCursorShapeManagerV1, State> for ManagerData {
    fn request(
        &self,
        state: &mut State,
        client: &Client,
        resource: &WpCursorShapeManagerV1,
        request: Request,
        handle: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        if let Request::GetPointer { ref pointer, .. } = request {
            let pointer_handle = PointerHandle::<State>::from_resource(pointer);
            let peer = state.niri.agent_seats.iter().find_map(|(peer, agent)| {
                (agent.seat.get_pointer().as_ref() == pointer_handle.as_ref()).then_some(*peer)
            });
            if let Some(peer) = peer {
                state
                    .with_agent(peer, |state, _agent| {
                        GlobalData.request(state, client, resource, request, handle, data_init);
                    })
                    .unwrap();
                return;
            }
            if state.niri.seat.get_pointer().as_ref() != pointer_handle.as_ref() {
                if let Request::GetPointer {
                    cursor_shape_device,
                    ..
                } = request
                {
                    data_init.init(cursor_shape_device, RetiredPointer);
                }
                return;
            }
        }
        GlobalData.request(state, client, resource, request, handle, data_init);
    }
}

impl Dispatch2<WpCursorShapeDeviceV1, State> for RetiredPointer {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _resource: &WpCursorShapeDeviceV1,
        _request: <WpCursorShapeDeviceV1 as Resource>::Request,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
    }
}
