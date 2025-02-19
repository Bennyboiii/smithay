//! Pointer-related types for smithay's input abstraction

use std::{
    fmt,
    sync::{Arc, Mutex},
};

use crate::{
    backend::input::{Axis, AxisSource, ButtonState},
    input::{Seat, SeatHandler},
    utils::Serial,
    utils::{IsAlive, Logical, Point},
};

mod cursor_image;
pub use cursor_image::{CursorImageAttributes, CursorImageStatus, CursorImageSurfaceData};

mod grab;
use grab::{DefaultGrab, GrabStatus};
pub use grab::{GrabStartData, PointerGrab};

/// An handle to a pointer handler
///
/// It can be cloned and all clones manipulate the same internal state.
///
/// This handle gives you access to an interface to send pointer events to your
/// clients.
///
/// When sending events using this handle, they will be intercepted by a pointer
/// grab if any is active. See the [`PointerGrab`] trait for details.
pub struct PointerHandle<D: SeatHandler> {
    pub(crate) inner: Arc<Mutex<PointerInternal<D>>>,
    #[cfg(feature = "wayland_frontend")]
    pub(crate) known_pointers: Arc<Mutex<Vec<wayland_server::protocol::wl_pointer::WlPointer>>>,
    #[cfg(feature = "wayland_frontend")]
    pub(crate) known_relative_pointers: Arc<Mutex<Vec<wayland_protocols::wp::relative_pointer::zv1::server::zwp_relative_pointer_v1::ZwpRelativePointerV1>>>,
}

#[cfg(not(feature = "wayland_frontend"))]
impl<D: SeatHandler> fmt::Debug for PointerHandle<D>
where
    <D as SeatHandler>::PointerFocus: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PointerHandle")
            .field("inner", &self.inner)
            .finish()
    }
}

#[cfg(feature = "wayland_frontend")]
impl<D: SeatHandler> fmt::Debug for PointerHandle<D>
where
    <D as SeatHandler>::PointerFocus: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PointerHandle")
            .field("inner", &self.inner)
            .field("known_pointers", &self.known_pointers)
            .field("known_relative_pointers", &self.known_relative_pointers)
            .finish()
    }
}

impl<D: SeatHandler> Clone for PointerHandle<D> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            #[cfg(feature = "wayland_frontend")]
            known_pointers: self.known_pointers.clone(),
            #[cfg(feature = "wayland_frontend")]
            known_relative_pointers: self.known_relative_pointers.clone(),
        }
    }
}

impl<D: SeatHandler> ::std::cmp::PartialEq for PointerHandle<D> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

/// Trait representing object that can receive pointer interactions
pub trait PointerTarget<D>: IsAlive + PartialEq + Clone + Send
where
    D: SeatHandler,
{
    /// A pointer of a given seat entered this handler
    fn enter(&self, seat: &Seat<D>, data: &mut D, event: &MotionEvent);
    /// A pointer of a given seat moved over this handler
    fn motion(&self, seat: &Seat<D>, data: &mut D, event: &MotionEvent);
    /// A pointer of a given seat that provides relative motion moved over this handler
    fn relative_motion(&self, seat: &Seat<D>, data: &mut D, event: &RelativeMotionEvent);
    /// A pointer of a given seat clicked a button
    fn button(&self, seat: &Seat<D>, data: &mut D, event: &ButtonEvent);
    /// A pointer of a given seat scrolled on an axis
    fn axis(&self, seat: &Seat<D>, data: &mut D, frame: AxisFrame);
    /// A pointer of a given seat left this handler
    fn leave(&self, seat: &Seat<D>, data: &mut D, serial: Serial, time: u32);
}

impl<D: SeatHandler + 'static> PointerHandle<D> {
    pub(crate) fn new() -> PointerHandle<D> {
        PointerHandle {
            inner: Arc::new(Mutex::new(PointerInternal::new())),
            #[cfg(feature = "wayland_frontend")]
            known_pointers: Arc::new(Mutex::new(Vec::new())),
            #[cfg(feature = "wayland_frontend")]
            known_relative_pointers: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Change the current grab on this pointer to the provided grab
    ///
    /// If focus is set to [`Focus::Clear`] any currently focused surface will be unfocused.
    ///
    /// Overwrites any current grab.
    pub fn set_grab<G: PointerGrab<D> + 'static>(&self, data: &mut D, grab: G, serial: Serial, focus: Focus) {
        let seat = self.get_seat(data);
        self.inner
            .lock()
            .unwrap()
            .set_grab(data, &seat, serial, grab, focus);
    }

    /// Remove any current grab on this pointer, resetting it to the default behavior
    pub fn unset_grab(&self, data: &mut D, serial: Serial, time: u32) {
        let seat = self.get_seat(data);
        self.inner.lock().unwrap().unset_grab(data, &seat, serial, time);
    }

    /// Check if this pointer is currently grabbed with this serial
    pub fn has_grab(&self, serial: Serial) -> bool {
        let guard = self.inner.lock().unwrap();
        match guard.grab {
            GrabStatus::Active(s, _) => s == serial,
            _ => false,
        }
    }

    /// Check if this pointer is currently being grabbed
    pub fn is_grabbed(&self) -> bool {
        let guard = self.inner.lock().unwrap();
        !matches!(guard.grab, GrabStatus::None)
    }

    /// Returns the start data for the grab, if any.
    pub fn grab_start_data(&self) -> Option<GrabStartData<D>> {
        let guard = self.inner.lock().unwrap();
        match &guard.grab {
            GrabStatus::Active(_, g) => Some(g.start_data().clone()),
            _ => None,
        }
    }

    /// Notify that the pointer moved
    ///
    /// You provide the new location of the pointer, in the form of:
    ///
    /// - The coordinates of the pointer in the global compositor space
    /// - The surface on top of which the cursor is, and the coordinates of its
    ///   origin in the global compositor space (or `None` of the pointer is not
    ///   on top of a client surface).
    ///
    /// This will internally take care of notifying the appropriate client objects
    /// of enter/motion/leave events.
    pub fn motion(
        &self,
        data: &mut D,
        focus: Option<(<D as SeatHandler>::PointerFocus, Point<i32, Logical>)>,
        event: &MotionEvent,
    ) {
        let mut inner = self.inner.lock().unwrap();
        inner.pending_focus = focus.clone();
        let seat = self.get_seat(data);
        inner.with_grab(&seat, move |mut handle, grab| {
            grab.motion(data, &mut handle, focus, event);
        });
    }

    /// Notify about relative pointer motion
    ///
    /// This will internally send the appropriate button event to the client
    /// objects matching with the currently focused surface, if the client uses
    /// the relative pointer protocol.
    pub fn relative_motion(
        &self,
        data: &mut D,
        focus: Option<(<D as SeatHandler>::PointerFocus, Point<i32, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        let mut inner = self.inner.lock().unwrap();
        inner.pending_focus = focus.clone();
        let seat = self.get_seat(data);
        inner.with_grab(&seat, move |mut handle, grab| {
            grab.relative_motion(data, &mut handle, focus, event);
        });
    }

    /// Notify that a button was pressed
    ///
    /// This will internally send the appropriate button event to the client
    /// objects matching with the currently focused surface.
    pub fn button(&self, data: &mut D, event: &ButtonEvent) {
        let mut inner = self.inner.lock().unwrap();
        match event.state {
            ButtonState::Pressed => {
                inner.pressed_buttons.push(event.button);
            }
            ButtonState::Released => {
                inner.pressed_buttons.retain(|b| *b != event.button);
            }
        }
        let seat = self.get_seat(data);
        inner.with_grab(&seat, |mut handle, grab| {
            grab.button(data, &mut handle, event);
        });
    }

    /// Start an axis frame
    ///
    /// A single frame will group multiple scroll events as if they happened in the same instance.
    pub fn axis(&self, data: &mut D, details: AxisFrame) {
        let seat = self.get_seat(data);
        self.inner.lock().unwrap().with_grab(&seat, |mut handle, grab| {
            grab.axis(data, &mut handle, details);
        });
    }

    /// Access the current location of this pointer in the global space
    pub fn current_location(&self) -> Point<f64, Logical> {
        self.inner.lock().unwrap().location
    }

    fn get_seat(&self, data: &mut D) -> Seat<D> {
        let seat_state = data.seat_state();
        seat_state
            .seats
            .iter()
            .find(|seat| seat.get_pointer().map(|h| &h == self).unwrap_or(false))
            .cloned()
            .unwrap()
    }
}

impl<D> PointerHandle<D>
where
    D: SeatHandler,
    <D as SeatHandler>::PointerFocus: Clone,
{
    /// Retrieve the current pointer focus
    pub fn current_focus(&self) -> Option<<D as SeatHandler>::PointerFocus> {
        self.inner.lock().unwrap().focus.clone().map(|(focus, _)| focus)
    }
}

/// This inner handle is accessed from inside a pointer grab logic, and directly
/// sends event to the client
pub struct PointerInnerHandle<'a, D: SeatHandler> {
    inner: &'a mut PointerInternal<D>,
    seat: &'a Seat<D>,
}

impl<'a, D: SeatHandler> fmt::Debug for PointerInnerHandle<'a, D>
where
    <D as SeatHandler>::PointerFocus: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PointerInnerHandle")
            .field("inner", &self.inner)
            .field("seat", &self.seat.arc.name)
            .finish()
    }
}

impl<'a, D: SeatHandler + 'static> PointerInnerHandle<'a, D> {
    /// Change the current grab on this pointer to the provided grab
    ///
    /// Overwrites any current grab.
    pub fn set_grab<G: PointerGrab<D> + 'static>(
        &mut self,
        data: &mut D,
        serial: Serial,
        focus: Focus,
        grab: G,
    ) {
        self.inner.set_grab(data, self.seat, serial, grab, focus);
    }

    /// Remove any current grab on this pointer, resetting it to the default behavior
    ///
    /// This will also restore the focus of the underlying pointer
    pub fn unset_grab(&mut self, data: &mut D, serial: Serial, time: u32) {
        self.inner.unset_grab(data, self.seat, serial, time);
    }

    /// Access the current focus of this pointer
    pub fn current_focus(&self) -> Option<(<D as SeatHandler>::PointerFocus, Point<i32, Logical>)> {
        self.inner.focus.clone()
    }

    /// Access the current location of this pointer in the global space
    pub fn current_location(&self) -> Point<f64, Logical> {
        self.inner.location
    }

    /// A list of the currently physically pressed buttons
    ///
    /// This still includes buttons that your grab have intercepted and not sent
    /// to the client.
    pub fn current_pressed(&self) -> &[u32] {
        &self.inner.pressed_buttons
    }

    /// Notify that the pointer moved
    ///
    /// You provide the new location of the pointer, in the form of:
    ///
    /// - The coordinates of the pointer in the global compositor space
    /// - The surface on top of which the cursor is, and the coordinates of its
    ///   origin in the global compositor space (or `None` of the pointer is not
    ///   on top of a client surface).
    ///
    /// This will internally take care of notifying the appropriate client objects
    /// of enter/motion/leave events.
    pub fn motion(
        &mut self,
        data: &mut D,
        focus: Option<(<D as SeatHandler>::PointerFocus, Point<i32, Logical>)>,
        event: &MotionEvent,
    ) {
        self.inner.motion(data, self.seat, focus, event);
    }

    /// Notify about relative pointer motion
    ///
    /// This will internally send the appropriate button event to the client
    /// objects matching with the currently focused surface, if the client uses
    /// the relative pointer protocol.
    pub fn relative_motion(
        &mut self,
        data: &mut D,
        focus: Option<(<D as SeatHandler>::PointerFocus, Point<i32, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        self.inner.relative_motion(data, self.seat, focus, event);
    }

    /// Notify that a button was pressed
    ///
    /// This will internally send the appropriate button event to the client
    /// objects matching with the currently focused surface.
    pub fn button(&mut self, data: &mut D, event: &ButtonEvent) {
        if let Some((focused, _)) = self.inner.focus.as_mut() {
            focused.button(self.seat, data, event);
        }
    }

    /// Notify that an axis was scrolled
    ///
    /// This will internally send the appropriate axis events to the client
    /// objects matching with the currently focused surface.
    pub fn axis(&mut self, data: &mut D, details: AxisFrame) {
        if let Some((focused, _)) = self.inner.focus.as_mut() {
            focused.axis(self.seat, data, details);
        }
    }
}

pub(crate) struct PointerInternal<D: SeatHandler> {
    pub(crate) focus: Option<(<D as SeatHandler>::PointerFocus, Point<i32, Logical>)>,
    pending_focus: Option<(<D as SeatHandler>::PointerFocus, Point<i32, Logical>)>,
    location: Point<f64, Logical>,
    grab: GrabStatus<D>,
    pressed_buttons: Vec<u32>,
}

// image_callback does not implement debug, so we have to impl Debug manually
impl<D: SeatHandler> fmt::Debug for PointerInternal<D>
where
    <D as SeatHandler>::PointerFocus: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PointerInternal")
            .field("focus", &self.focus)
            .field("pending_focus", &self.pending_focus)
            .field("location", &self.location)
            .field("grab", &self.grab)
            .field("pressed_buttons", &self.pressed_buttons)
            .field("image_callback", &"...")
            .finish()
    }
}

impl<D: SeatHandler + 'static> PointerInternal<D> {
    fn new() -> Self {
        Self {
            focus: None,
            pending_focus: None,
            location: (0.0, 0.0).into(),
            grab: GrabStatus::None,
            pressed_buttons: Vec::new(),
        }
    }

    fn set_grab<G: PointerGrab<D> + 'static>(
        &mut self,
        data: &mut D,
        seat: &Seat<D>,
        serial: Serial,
        grab: G,
        focus: Focus,
    ) {
        self.grab = GrabStatus::Active(serial, Box::new(grab));

        if matches!(focus, Focus::Clear) {
            let location = self.location;
            self.motion(
                data,
                seat,
                None,
                &MotionEvent {
                    location,
                    serial,
                    time: 0,
                },
            );
        }
    }

    fn unset_grab(&mut self, data: &mut D, seat: &Seat<D>, serial: Serial, time: u32) {
        self.grab = GrabStatus::None;
        // restore the focus
        let location = self.location;
        let focus = self.pending_focus.clone();
        self.motion(
            data,
            seat,
            focus,
            &MotionEvent {
                location,
                serial,
                time,
            },
        );
    }

    fn motion(
        &mut self,
        data: &mut D,
        seat: &Seat<D>,
        focus: Option<(<D as SeatHandler>::PointerFocus, Point<i32, Logical>)>,
        event: &MotionEvent,
    ) {
        // do we leave a surface ?
        let mut leave = true;
        self.location = event.location;
        if let Some((ref current_focus, _)) = self.focus {
            if let Some((ref new_focus, _)) = focus {
                if current_focus == new_focus {
                    leave = false;
                }
            }
        }
        if leave {
            if let Some((focused, _)) = self.focus.as_mut() {
                focused.leave(seat, data, event.serial, event.time);
            }
            self.focus = None;
            data.cursor_image(seat, CursorImageStatus::Default);
        }

        // do we enter one ?
        if let Some((surface, surface_location)) = focus {
            let entered = self.focus.is_none();
            // in all cases, update the focus, the coordinates of the surface
            // might have changed
            self.focus = Some((surface, surface_location));
            let event = MotionEvent {
                location: event.location - surface_location.to_f64(),
                serial: event.serial,
                time: event.time,
            };
            let (focused, _) = self.focus.as_mut().unwrap();
            if entered {
                focused.enter(seat, data, &event);
            } else {
                // we were on top of a surface and remained on it
                focused.motion(seat, data, &event);
            }
        }
    }

    fn relative_motion(
        &mut self,
        data: &mut D,
        seat: &Seat<D>,
        _focus: Option<(<D as SeatHandler>::PointerFocus, Point<i32, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        if let Some((focused, _)) = self.focus.as_mut() {
            focused.relative_motion(seat, data, event);
        }
    }

    fn with_grab<F>(&mut self, seat: &Seat<D>, f: F)
    where
        F: FnOnce(PointerInnerHandle<'_, D>, &mut dyn PointerGrab<D>),
    {
        let mut grab = ::std::mem::replace(&mut self.grab, GrabStatus::Borrowed);
        match grab {
            GrabStatus::Borrowed => panic!("Accessed a pointer grab from within a pointer grab access."),
            GrabStatus::Active(_, ref mut handler) => {
                // If this grab is associated with a surface that is no longer alive, discard it
                if let Some((ref focus, _)) = handler.start_data().focus {
                    if !focus.alive() {
                        self.grab = GrabStatus::None;
                        f(PointerInnerHandle { inner: self, seat }, &mut DefaultGrab);
                        return;
                    }
                }
                f(PointerInnerHandle { inner: self, seat }, &mut **handler);
            }
            GrabStatus::None => {
                f(PointerInnerHandle { inner: self, seat }, &mut DefaultGrab);
            }
        }

        if let GrabStatus::Borrowed = self.grab {
            // the grab has not been ended nor replaced, put it back in place
            self.grab = grab;
        }
    }
}

/// Defines the focus behavior for [`PointerHandle::set_grab`]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Focus {
    /// Keep the current focus
    Keep,
    /// Clear the current focus
    Clear,
}

/// Pointer motion event
#[derive(Debug, Clone)]
pub struct MotionEvent {
    /// Location of the pointer in compositor space
    pub location: Point<f64, Logical>,
    /// Serial of the event
    pub serial: Serial,
    /// Timestamp of the event, with millisecond granularity
    pub time: u32,
}

/// Relative pointer motion event
#[derive(Debug, Clone)]
pub struct RelativeMotionEvent {
    /// Motional vector
    pub delta: Point<f64, Logical>,
    /// Unaccelerated motion vector
    pub delta_unaccel: Point<f64, Logical>,
    /// Timestamp in microseconds
    pub utime: u64,
}

/// Pointer button event

/// Mouse button click and release notifications.
/// The location of the click is given by the last motion or enter event.
#[derive(Debug, Clone, Copy)]
pub struct ButtonEvent {
    /// Serial of the event
    pub serial: Serial,
    /// Timestamp with millisecond granularity, with an undefined base.
    pub time: u32,
    /// Button that produced the event
    ///
    /// The button is a button code as defined in the
    /// Linux kernel's linux/input-event-codes.h header file, e.g. BTN_LEFT.
    ///
    /// Any 16-bit button code value is reserved for future additions to the kernel's event code list. All other button codes above 0xFFFF are currently undefined but may be used in future versions of this protocol.
    pub button: u32,
    /// Physical state of the button
    pub state: ButtonState,
}

/// A frame of pointer axis events.
/// Frames of axis events should be considers as one logical action.
///
/// Can be used with the builder pattern, e.g.:
///
/// ```ignore
/// AxisFrame::new()
///     .source(AxisSource::Wheel)
///     .discrete(Axis::Vertical, 6)
///     .value(Axis::Vertical, 30, time)
///     .stop(Axis::Vertical);
/// ```
#[derive(Copy, Clone, Debug)]
pub struct AxisFrame {
    /// Source of the axis event, if known
    pub source: Option<AxisSource>,
    /// Time of the axis event
    pub time: u32,
    /// Raw scroll value per axis of the event
    pub axis: (f64, f64),
    /// Discrete representation of scroll value per axis, if available
    pub discrete: Option<(i32, i32)>,
    /// If the axis is considered having stoped movement
    ///
    /// Only useful in conjunction of AxisSource::Finger events
    pub stop: (bool, bool),
}

impl AxisFrame {
    /// Create a new frame of axis events
    pub fn new(time: u32) -> Self {
        AxisFrame {
            source: None,
            time,
            axis: (0.0, 0.0),
            discrete: None,
            stop: (false, false),
        }
    }

    /// Specify the source of the axis events
    ///
    /// This event is optional, if no source is known, you can ignore this call.
    /// Only one source event is allowed per frame.
    ///
    /// Using the [`AxisSource::Finger`] requires a stop event to be send,
    /// when the user lifts off the finger (not necessarily in the same frame).
    pub fn source(mut self, source: AxisSource) -> Self {
        self.source = Some(source);
        self
    }

    /// Specify discrete scrolling steps additionally to the computed value.
    ///
    /// This event is optional and gives the client additional information about
    /// the nature of the axis event. E.g. a scroll wheel might issue separate steps,
    /// while a touchpad may never issue this event as it has no steps.
    pub fn discrete(mut self, axis: Axis, steps: i32) -> Self {
        let mut discrete = self.discrete.get_or_insert_with(Default::default);
        match axis {
            Axis::Horizontal => {
                discrete.0 = steps;
            }
            Axis::Vertical => {
                discrete.1 = steps;
            }
        };
        self
    }

    /// The actual scroll value. This event is the only required one, but can also
    /// be send multiple times. The values off one frame will be accumulated by the client.
    pub fn value(mut self, axis: Axis, value: f64) -> Self {
        match axis {
            Axis::Horizontal => {
                self.axis.0 += value;
            }
            Axis::Vertical => {
                self.axis.1 += value;
            }
        };
        self
    }

    /// Notification of stop of scrolling on an axis.
    ///
    /// This event is required for sources of the [`AxisSource::Finger`] type
    /// and otherwise optional.
    pub fn stop(mut self, axis: Axis) -> Self {
        match axis {
            Axis::Horizontal => {
                self.stop.0 = true;
            }
            Axis::Vertical => {
                self.stop.1 = true;
            }
        };
        self
    }
}
