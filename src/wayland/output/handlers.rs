use slog::{trace, warn};
use wayland_protocols::xdg::xdg_output::zv1::server::{
    zxdg_output_manager_v1::{self, ZxdgOutputManagerV1},
    zxdg_output_v1::{self, ZxdgOutputV1},
};
use wayland_server::{
    protocol::wl_output::{self, Mode as WMode, WlOutput},
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

use super::{xdg::XdgOutput, Output, OutputManagerState, OutputUserData, WlOutputData};

/*
 * Wl Output
 */

impl<D> GlobalDispatch<WlOutput, WlOutputData, D> for OutputManagerState
where
    D: GlobalDispatch<WlOutput, WlOutputData>,
    D: Dispatch<WlOutput, OutputUserData>,
    D: 'static,
{
    fn bind(
        _state: &mut D,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<WlOutput>,
        global_data: &WlOutputData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let output = data_init.init(
            resource,
            OutputUserData {
                global_data: global_data.inner.clone(),
            },
        );

        let mut inner = global_data.inner.0.lock().unwrap();

        trace!(inner.log, "New WlOutput global instantiated."; "name" => &inner.name);

        if inner.modes.is_empty() {
            warn!(inner.log, "Output is used with no modes set"; "name" => &inner.name);
        }
        if inner.current_mode.is_none() {
            warn!(inner.log, "Output is used with no current mod set"; "name" => &inner.name);
        }
        if inner.preferred_mode.is_none() {
            warn!(inner.log, "Output is used with not preferred mode set"; "name" => &inner.name);
        }

        inner.send_geometry_to(&output);

        for &mode in &inner.modes {
            let mut flags = WMode::empty();
            if Some(mode) == inner.current_mode {
                flags |= WMode::Current;
            }
            if Some(mode) == inner.preferred_mode {
                flags |= WMode::Preferred;
            }
            output.mode(flags, mode.size.w, mode.size.h, mode.refresh);
        }

        if output.version() >= 4 {
            output.name(inner.name.clone());
            output.description(inner.description.clone())
        }

        if output.version() >= 2 {
            output.scale(inner.scale.integer_scale());
            output.done();
        }

        inner.instances.push(output);
    }
}

impl<D> Dispatch<WlOutput, OutputUserData, D> for OutputManagerState
where
    D: Dispatch<WlOutput, OutputUserData>,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _resource: &WlOutput,
        _request: wl_output::Request,
        _data: &OutputUserData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
    }

    fn destroyed(
        _state: &mut D,
        _client_id: wayland_server::backend::ClientId,
        object_id: wayland_server::backend::ObjectId,
        data: &OutputUserData,
    ) {
        data.global_data
            .0
            .lock()
            .unwrap()
            .instances
            .retain(|o| o.id() != object_id);
    }
}

/*
 * XDG Output
 */

impl<D> GlobalDispatch<ZxdgOutputManagerV1, (), D> for OutputManagerState
where
    D: GlobalDispatch<ZxdgOutputManagerV1, ()>,
    D: Dispatch<ZxdgOutputManagerV1, ()>,
    D: Dispatch<ZxdgOutputV1, XdgOutputUserData>,
    D: 'static,
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZxdgOutputManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }
}

impl<D> Dispatch<ZxdgOutputManagerV1, (), D> for OutputManagerState
where
    D: Dispatch<ZxdgOutputManagerV1, ()>,
    D: Dispatch<ZxdgOutputV1, XdgOutputUserData>,
    D: 'static,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _resource: &ZxdgOutputManagerV1,
        request: zxdg_output_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zxdg_output_manager_v1::Request::GetXdgOutput {
                id,
                output: wl_output,
            } => {
                let output = Output::from_resource(&wl_output).unwrap();
                let mut inner = output.inner.0.lock().unwrap();

                let xdg_output = XdgOutput::new(&inner, inner.log.clone());

                if inner.xdg_output.is_none() {
                    inner.xdg_output = Some(xdg_output.clone());
                }

                let id = data_init.init(id, XdgOutputUserData { xdg_output });

                inner.xdg_output.as_ref().unwrap().add_instance(&id, &wl_output);
            }
            zxdg_output_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

/// User data of Xdg Output
#[derive(Debug)]
pub struct XdgOutputUserData {
    xdg_output: XdgOutput,
}

impl<D> Dispatch<ZxdgOutputV1, XdgOutputUserData, D> for OutputManagerState
where
    D: Dispatch<ZxdgOutputV1, XdgOutputUserData>,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _resource: &ZxdgOutputV1,
        _request: zxdg_output_v1::Request,
        _data: &XdgOutputUserData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
    }

    fn destroyed(
        _state: &mut D,
        _client_id: wayland_server::backend::ClientId,
        object_id: wayland_server::backend::ObjectId,
        data: &XdgOutputUserData,
    ) {
        data.xdg_output
            .inner
            .lock()
            .unwrap()
            .instances
            .retain(|o| o.id() != object_id);
    }
}
