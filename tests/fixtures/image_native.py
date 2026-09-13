"""Native image composer stand-in: retain exact input, never submit anything."""
import os,pathlib,select,sys,termios,tty
root=pathlib.Path.cwd();old=termios.tcgetattr(0)
try:
 tty.setraw(0);(root/'image-input').write_bytes(b'')
 # A native attempt to write the local clipboard must stay inert in the emulator.
 sys.stdout.write('\x1bPq#1~\x1b\\\x1b]52;c;Tk9fQ0xJUEJPQVJE\x07\x1b[?2004h\x1b[2J\x1b[HIMAGE_NATIVE_DRAFT');sys.stdout.flush()
 display=root/'image-display';displayed=False
 while True:
  if display.exists() and not displayed:
   os.write(1,display.read_bytes()[:8192]);displayed=True
  if select.select([0],[],[],.05)[0]:
   b=os.read(0,4096)
   if not b:break
   with (root/'image-input').open('ab') as f:f.write(b)
finally:termios.tcsetattr(0,termios.TCSANOW,old)
