use std::any::{Any, TypeId};
use std::default::Default;
use std::marker::PhantomData;
use std::mem::transmute;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;

use crate::container_of::ContainerOf;
use crate::winapi::OVERLAPPED;

// Wrapper around OVERLAPPED.
// mio expects all events that arrive on it's completion port to be wrapped with this.
#[derive(Default)]
pub struct EventState {
  event_handler: Option<Box<dyn EventHandler>>,
  overlapped: OVERLAPPED,
}

impl ContainerOf<OVERLAPPED> for EventState {
  fn member(&self) -> &OVERLAPPED {
    &self.overlapped
  }
}

impl EventState {
  pub fn new() -> Self {
    Default::default()
  }

  fn as_overlapped(&mut self) -> NonNull<OVERLAPPED> {
    let overlapped = &mut self.overlapped;
    overlapped.into()
  }

  unsafe fn from_overlapped(
    overlapped: NonNull<OVERLAPPED>,
  ) -> &'static mut Self {
    let overlapped = &mut *overlapped.as_ptr();
    Self::container_of_mut(overlapped)
  }

  // Embed ownership of the EventHandler inside its own EventState, and then
  // This reference cycle violates Rust borrowing rules, so we make both the state
  // and handler inaccessible by returning a raw pointer to the win32 OVERLAPPED struct.
  fn embed_event_handler(
    mut event_handler: Box<dyn EventHandler>,
  ) -> NonNull<OVERLAPPED> {
    let state: &mut Self = event_handler.state();
    assert!(state.event_handler.is_none());
    let state: &'static mut Self = unsafe { transmute(state) };
    state.event_handler = Some(event_handler);
    state.as_overlapped()
  }

  // The OVERLAPPED can and must be converted back to an EventHandler exactly once.
  // At this point the reference cycle is also broken and ownership of the event handler
  // is returned to the caller.
  unsafe fn extract_event_handler(
    overlapped: NonNull<OVERLAPPED>,
  ) -> Box<dyn EventHandler> {
    let state = Self::from_overlapped(overlapped);
    state.event_handler.take().unwrap()
  }

  fn downcast_event_handler<T>(event_handler: Box<dyn EventHandler>) -> Box<T>
  where
    T: EventHandler,
  {
    assert!((*event_handler).type_id() == TypeId::of::<T>());
    let ptr = Box::into_raw(event_handler) as *mut T;
    unsafe { Box::from_raw(ptr) }
  }

  // Must be called by the user before starting an overlapped operation.
  // Returns a Dispatch object, that has two purposes:
  //   * It has an `overlapped()` method to retrieve the pointer win32 apis expect.
  //   * It has `pending()` and `failed()` methods. The user is expected to call
  //     either of those to indicate whether the overlapped i/o operation
  //     was successfully started.
  fn dispatch<T>(event_handler: Box<T>) -> Dispatch<T>
  where
    T: EventHandler,
  {
    Dispatch::<T>::new(Self::embed_event_handler(event_handler))
    // TODO: notify MIO here that it should expect an overlapped
    // completion event to show up on the completion port eventually.
  }

  // If the windows API indicated failure, this function can be used to turn the raw
  // *mut OVERLAPPED back into the original boxed event handler. This is not called
  // by the user directly, but by the implementation of `struct Dispatch`.
  unsafe fn undispatch<T>(overlapped: NonNull<OVERLAPPED>) -> Box<T>
  where
    T: EventHandler,
  {
    let handler = Self::extract_event_handler(overlapped);
    Self::downcast_event_handler(handler)
    // TODO: notify MIO here that some event isn't coming after all.
  }

  // Called by mio when the OVERLAPPED was returned by GetQueuedCompletionStatusEx()
  pub unsafe fn complete(overlapped: NonNull<OVERLAPPED>) -> () {
    let handler = Self::extract_event_handler(overlapped);
    handler.complete()
  }
}

impl Deref for EventState {
  type Target = OVERLAPPED;

  fn deref(&self) -> &Self::Target {
    &self.overlapped
  }
}

impl DerefMut for EventState {
  fn deref_mut(&mut self) -> &mut Self::Target {
    &mut self.overlapped
  }
}

pub struct Dispatch<T> {
  overlapped: Option<NonNull<OVERLAPPED>>,
  _phantom: PhantomData<T>,
}

impl<T> Dispatch<T>
where
  T: EventHandler,
{
  fn new(overlapped: NonNull<OVERLAPPED>) -> Self {
    Self {
      overlapped: Some(overlapped),
      _phantom: PhantomData,
    }
  }

  pub fn pending(mut self) -> () {
    // In a multi-threaded scenario, the overlapped event might complete and
    // be picked up by another thread *before* `pending()` is called. So it is
    // paramount never to convert `self.overlapped` back to an EventState!
    self.overlapped.take().unwrap();
  }

  pub fn failed(mut self) -> Box<T> {
    let overlapped = self.overlapped.take().unwrap();
    unsafe { EventState::undispatch(overlapped) }
  }

  pub fn overlapped(&mut self) -> *mut OVERLAPPED {
    self.overlapped.unwrap().as_ptr()
  }
}

impl<T> Drop for Dispatch<T> {
  fn drop(&mut self) {
    if self.overlapped.is_some() {
      panic!("Either Dispatch::pending() or Dispatch::failed() must be called after dispatching an EventState.");
    }
  }
}

// IOCP 'plug-ins' like wepoll, mio_named_pipes, etc... implement this trait.
pub trait EventHandler
where
  Self: Any + Send + 'static,
{
  fn state(&mut self) -> &mut EventState;
  fn complete(self: Box<Self>) -> ();
}

// Helper trait that allows the user to call `dispatch()` on any object that
// implements EventHandler.
pub trait EventDispatch<T> {
  #[must_use]
  fn dispatch(self: Box<Self>) -> Dispatch<T>;
}
impl<T> EventDispatch<T> for T
where
  T: EventHandler,
{
  fn dispatch(self: Box<Self>) -> Dispatch<T> {
    EventState::dispatch(self)
  }
}
