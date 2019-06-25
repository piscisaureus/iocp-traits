use std::any::{Any, TypeId};
use std::default::Default;
use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::marker::PhantomData;
use std::mem::transmute;
use std::mem::{align_of, size_of};
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;

mod winapi {
  // Not the real thing, but writing this on mac...
  #[derive(Default)]
  pub struct OVERLAPPED {
    _foo: i64,
    _bar: i32,
  }
  unsafe impl Send for OVERLAPPED {}
}

trait ContainerOf<T>
where
  Self: Sized + 'static,
  T: Sized + 'static,
{
  fn member(&self) -> &T;

  #[inline(always)]
  unsafe fn member_offset() -> usize {
    let dummy = NonNull::<Self>::dangling();

    let container: &Self = dummy.as_ref();
    let container_start_addr = container as *const _ as usize;
    let container_end_addr = container_start_addr + size_of::<Self>();

    let member = container.member();
    let member_start_addr = member as *const _ as usize;
    let member_end_addr = member_start_addr + size_of::<T>();

    assert!(container_start_addr <= member_start_addr);
    assert!(container_end_addr >= member_end_addr);

    member_start_addr - container_start_addr
  }

  #[inline(always)]
  unsafe fn container_of_ptr(member: *const T) -> *const Self {
    let member_addr = member as usize;
    let member_offset = Self::member_offset();
    assert!(member_addr > member_offset);

    let container_addr = member_addr - member_offset;
    let container_alignment = align_of::<Self>();
    assert!(container_addr % container_alignment == 0);

    container_addr as *const Self
  }

  #[inline(always)]
  unsafe fn container_of(member: &T) -> &Self {
    let member_ptr = member as *const T;
    let container_ptr = Self::container_of_ptr(member_ptr);
    &*container_ptr
  }

  #[inline(always)]
  unsafe fn container_of_mut(member: &mut T) -> &mut Self {
    let member_ptr = member as *mut _ as *const T;
    let container_ptr = Self::container_of_ptr(member_ptr) as *mut Self;
    &mut *container_ptr
  }
}

// Wrapper around winapi::OVERLAPPED.
// mio expects all events that arrive on it's completion port to be wrapped with this.
#[derive(Default)]
pub struct EventState {
  event_handler: Option<Box<dyn EventHandler>>,
  overlapped: winapi::OVERLAPPED,
}

impl ContainerOf<winapi::OVERLAPPED> for EventState {
  fn member(&self) -> &winapi::OVERLAPPED {
    &self.overlapped
  }
}

impl EventState {
  pub fn new() -> Self {
    Default::default()
  }

  fn as_overlapped(&mut self) -> NonNull<winapi::OVERLAPPED> {
    let overlapped = &mut self.overlapped;
    overlapped.into()
  }

  unsafe fn from_overlapped(
    overlapped: NonNull<winapi::OVERLAPPED>,
  ) -> &'static mut Self {
    let overlapped = &mut *overlapped.as_ptr();
    Self::container_of_mut(overlapped)
  }

  // Embed ownership of the EventHandler inside its own EventState, and then
  // This reference cycle violates Rust borrowing rules, so we make both the state
  // and handler inaccessible by returning a raw pointer to the win32 OVERLAPPED struct.
  fn embed_event_handler(
    mut event_handler: Box<dyn EventHandler>,
  ) -> NonNull<winapi::OVERLAPPED> {
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
    overlapped: NonNull<winapi::OVERLAPPED>,
  ) -> Box<dyn EventHandler> {
    let state = Self::from_overlapped(overlapped);
    state.event_handler.take().unwrap()
  }

  // Must be called by the user when starting an overlapped operation.
  // Returns the raw *OVERLAPPED pointer that windows APIs expect.
  pub fn dispatch<T>(event_handler: Box<T>) -> Dispatch<T>
  where
    T: EventHandler,
  {
    let dyn_event_handler = event_handler as Box<dyn EventHandler>;
    Dispatch::<T>::new(Self::embed_event_handler(dyn_event_handler))
  }

  // If the windows API indicated failure, this function can be used to turn the raw
  // *mut OVERLAPPED back into the original boxed event handler. This is not called
  // by the user directly, but by the implementation of `struct Dispatch`.
  unsafe fn undispatch<T>(overlapped: NonNull<winapi::OVERLAPPED>) -> Box<T>
  where
    T: EventHandler,
  {
    let handler = Self::extract_event_handler(overlapped);
    // Convert back from trait object to concrete type.
    assert!((*handler).type_id() == TypeId::of::<T>());
    let handler = Box::into_raw(handler) as *mut T;
    let handler: Box<T> = Box::from_raw(handler);
    // Here, notify mio that some event isn't coming after all..
    // ...
    handler
  }

  // Called by mio when the OVERLAPPED was returned by GetQueuedCompletionStatusEx()
  pub unsafe fn complete(overlapped: NonNull<winapi::OVERLAPPED>) -> () {
    let handler = Self::extract_event_handler(overlapped);
    handler.complete()
  }
}

impl Deref for EventState {
  type Target = winapi::OVERLAPPED;

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
  overlapped: Option<NonNull<winapi::OVERLAPPED>>,
  _phantom: PhantomData<T>,
}

impl<T> Dispatch<T>
where
  T: EventHandler,
{
  fn new(overlapped: NonNull<winapi::OVERLAPPED>) -> Self {
    Self {
      overlapped: Some(overlapped.into()),
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

  pub fn overlapped(&mut self) -> *mut winapi::OVERLAPPED {
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
    EventState::dispatch::<T>(self)
  }
}

// Sample usage.
struct AfdPoll {
  state: EventState,
  bits: u32,
}

impl EventHandler for AfdPoll {
  fn state(&mut self) -> &mut EventState {
    &mut self.state
  }

  fn complete(self: Box<Self>) -> () {
    println!("AfdPoll event, bits: {}", self.bits);
  }
}

impl Debug for AfdPoll {
  fn fmt(&self, f: &mut Formatter) -> FmtResult {
    write!(f, "AfdPoll {{ bits: {:?} }}", self.bits)
  }
}

// Sample usage.
struct PipeRead {
  text: &'static str,
  state: EventState,
}

impl EventHandler for PipeRead {
  fn state(&mut self) -> &mut EventState {
    &mut self.state
  }

  fn complete(self: Box<Self>) -> () {
    println!("PipeRead event, text: {}", self.text);
  }
}

fn main() -> () {
  let pipe_read_1 = Box::new(PipeRead {
    text: "foo",
    state: EventState::new(),
  });
  let pipe_read_2 = Box::new(PipeRead {
    text: "bar",
    state: EventState::new(),
  });
  let afd_poll_1 = Box::new(AfdPoll {
    bits: 22,
    state: EventState::new(),
  });
  let afd_poll_2 = Box::new(AfdPoll {
    bits: 1234,
    state: EventState::new(),
  });

  // How it would be used.
  /*
  let dispatch = pipe_read_1.dispatch();
  let result = ReadFile(pipe_handle, yadda, yadda, dispatch.overlapped());
  if result == TRUE || GetLastError() == ERROR_IO_PENDING {
      dispatch.pending();
  } else {
      dispatch.failed();
  }
  */

  // But in this test it's all phony.
  let mut succeeded_events: Vec<*mut winapi::OVERLAPPED> = Vec::new();
  let mut d = pipe_read_1.dispatch();
  succeeded_events.push(d.overlapped());
  d.pending();
  let mut d = pipe_read_2.dispatch();
  succeeded_events.push(d.overlapped());
  d.pending();
  let mut d = afd_poll_1.dispatch();
  succeeded_events.push(d.overlapped());
  d.pending();

  // Pretend this one fails.
  let d = afd_poll_2.dispatch();
  let event: Box<AfdPoll> = d.failed();
  println!("Dispatch failed for {:?}", event);

  for overlapped in succeeded_events {
    unsafe { EventState::complete(NonNull::new(overlapped).unwrap()) }
  }
}
