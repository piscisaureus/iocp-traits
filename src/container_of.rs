use std::mem::{align_of, size_of};
use std::ptr::NonNull;

pub trait ContainerOf<T>
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
