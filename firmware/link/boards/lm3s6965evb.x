/* The emulation build's layout for QEMU's `lm3s6965evb` machine (feature
   `board-lm3s6965evb`), which runs the thumbv7m-none-eabi images: TI's
   Stellaris LM3S6965, a Cortex-M3, with 256 KiB of flash at 0 and 64 KiB of
   RAM at 0x20000000, as `info mtree` shows them; the same sizes as the link
   test's generic layout. cortex-m-rt puts the stack at the top of RAM. */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 256K
  RAM   : ORIGIN = 0x20000000, LENGTH = 64K
}
