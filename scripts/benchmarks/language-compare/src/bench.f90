program language_compare
  use iso_fortran_env, only: int64, int8
  implicit none
  integer, parameter :: limit = 10000000
  integer(int8), allocatable :: marked(:)
  integer :: p, j, count
  integer(int64) :: hash, i
  allocate(marked(0:limit))
  marked = 0_int8
  p = 2
  do while (p*p <= limit)
     if (marked(p) == 0_int8) then
        j = p*p
        do while (j <= limit)
           marked(j) = 1_int8
           j = j + p
        end do
     end if
     p = p + 1
  end do
  count = 0
  do j = 2, limit
     if (marked(j) == 0_int8) count = count + 1
  end do
  deallocate(marked)
  hash = 2166136261_int64
  do i = 0_int64, 19999999_int64
     hash = iand(ieor(hash, iand(i*131_int64+17_int64, 255_int64))*16777619_int64, 4294967295_int64)
  end do
  print '(I0)', count
  print '(I0)', hash
end program language_compare
