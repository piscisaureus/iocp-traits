mod container_of;
mod iocp;
mod winapi;

use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::ptr::NonNull;

use crate::iocp::*;
use crate::winapi::OVERLAPPED;

// Sample usage -- AfdPoll is an 'iocp plugin'.
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

// Sample usage -- PipeRead is another 'iocp plugin'.
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
  let mut fake_iocp_results: Vec<*mut OVERLAPPED> = Vec::new();
  let mut d = pipe_read_1.dispatch();
  fake_iocp_results.push(d.overlapped());
  d.pending();
  let mut d = pipe_read_2.dispatch();
  fake_iocp_results.push(d.overlapped());
  d.pending();
  let mut d = afd_poll_1.dispatch();
  fake_iocp_results.push(d.overlapped());
  d.pending();

  // Pretend this one fails.
  let d = afd_poll_2.dispatch();
  let event: Box<AfdPoll> = d.failed();
  println!("Dispatch failed for {:?}", event);

  for overlapped in fake_iocp_results {
    unsafe { EventState::complete(NonNull::new(overlapped).unwrap()) }
  }
}
