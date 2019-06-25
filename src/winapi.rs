// Not the real thing, but writing this on mac...
#[derive(Default)]
pub struct OVERLAPPED {
  _foo: i64,
  _bar: i32,
}
unsafe impl Send for OVERLAPPED {}
