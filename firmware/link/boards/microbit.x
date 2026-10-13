/* The emulation build's layout for QEMU's `microbit` machine (feature
   `board-microbit`), which runs the thumbv6m-none-eabi images: a BBC
   micro:bit's nRF51822, a Cortex-M0, with 256 KiB of flash at 0 and 16 KiB
   of RAM at 0x20000000, as `info mtree` shows them. The smallest RAM of the
   boards: the MS/TP image's 5.5 KB of statics leave it about 10 KB of
   stack. cortex-m-rt puts the stack at the top of RAM. */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 256K
  RAM   : ORIGIN = 0x20000000, LENGTH = 16K
}
