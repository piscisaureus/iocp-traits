use std::ptr::NonNull;

// Why is this not in std? https://github.com/rust-lang/rust/issues/47336
trait CastNonNull<T>
where
  T: ?Sized,
{
  fn into_non_null(self) -> NonNull<T>;
  unsafe fn from_non_null(from: NonNull<T>) -> Self;
}

impl<T> CastNonNull<T> for Box<T>
where
  T: ?Sized,
{
  #[inline(always)]
  fn into_non_null(mut self) -> NonNull<T> {
    let ptr = Box::into_raw(self);
    unsafe { NonNull::new_unchecked(ptr) }
  }

  #[inline(always)]
  unsafe fn from_non_null(from: NonNull<T>) -> Self {
    let ptr = from.as_ptr();
    unsafe { Box::from_raw(ptr) }
  }
}
