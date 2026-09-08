#!/usr/bin/env python3
"""Native appearance smoke test; run from the Surfer root after cargo build.

Requires GNOME Shell with headless automation, gdbus, and dbus-run-session.
Uses a disposable compositor/configuration, never the current desktop. Captures
Atlas Light, the dropdown, Dracula, maximized, and 520x320 layouts under /tmp.
"""
import os, subprocess, time, tempfile
from pathlib import Path
root=Path(tempfile.mkdtemp(prefix='surfer-native-'))
os.chmod(root,0o700)
(root/'config/surfer').mkdir(parents=True)
(root/'automation.js').write_text('export function init() { global.context.unsafe_mode = true; }\nexport async function run() { await new Promise(() => {}); }\n')
(root/'config/surfer/config.toml').write_text('theme = \"Atlas Light\"\n[layout]\nwindow_width = 1100\nwindow_height = 700\n')
env=dict(os.environ,XDG_RUNTIME_DIR=str(root),XDG_CONFIG_HOME=str(root/'config'),XDG_CACHE_HOME=str(root/'cache'),XDG_DATA_HOME=str(root/'data'),LIBGL_ALWAYS_SOFTWARE='1',GSETTINGS_BACKEND='memory')
for name in ['DISPLAY','WAYLAND_DISPLAY','WAYLAND_SOCKET','DBUS_SESSION_BUS_ADDRESS']:
 env.pop(name,None)
worker=r'''
import os,subprocess,time
from pathlib import Path
root=Path(os.environ['XDG_RUNTIME_DIR'])
log=open('/tmp/surfer-compositor.log','w')
shell=subprocess.Popen(['gnome-shell','--wayland','--headless','--virtual-monitor','1600x1000','--wayland-display','surfer-test','--debug-control','--automation-script',str(root/'automation.js')],stdout=log,stderr=subprocess.STDOUT)
app=None
try:
 for _ in range(50):
  if (root/'surfer-test').exists():break
  if shell.poll() is not None:raise RuntimeError('Compositor exited')
  time.sleep(.2)
 env=dict(os.environ,WAYLAND_DISPLAY='surfer-test',WAYLAND_DEBUG='client')
 app=subprocess.Popen(['target/debug/surfer','examples/picorv32.vcd','-C','scope_add testbench.top; scope_select testbench.top.uut.picorv32_core; zoom_to 1000000 5000000; cursor_set 2000000'],env=env,stdout=open('/tmp/surfer-native-app.log','w'),stderr=subprocess.STDOUT)
 time.sleep(4)
 print('App status:',app.poll(),flush=True)
 def evaluate(script):
  result=subprocess.run(['gdbus','call','--session','--dest','org.gnome.Shell','--object-path','/org/gnome/Shell','--method','org.gnome.Shell.Eval',script],capture_output=True,text=True)
  assert result.returncode == 0 and result.stdout.startswith('(true,'), (script, result.stdout, result.stderr)
  print('Eval:',result.stdout,result.stderr,flush=True)
  return result.stdout
 evaluate("global.surferKeyboard = global.stage.context.get_backend().get_default_seat().create_virtual_device(imports.gi.Clutter.InputDeviceType.KEYBOARD_DEVICE); global.surferPointer = global.stage.context.get_backend().get_default_seat().create_virtual_device(imports.gi.Clutter.InputDeviceType.POINTER_DEVICE); global.surferPointer.notify_absolute_motion(imports.gi.GLib.get_monotonic_time(), 1500, 950)")
 time.sleep(.5)
 evaluate("global.surferWindow = global.get_window_actors().find(a => a.meta_window.get_title().includes('Surfer')).meta_window; global.surferWindow.unmaximize(3); global.surferWindow.move_resize_frame(false, 180, 170, 1100, 700); Main.overview.hide(); global.surferWindow.activate(global.get_current_time())")
 time.sleep(3)
 shot=['gdbus','call','--session','--dest','org.gnome.Shell.Screenshot','--object-path','/org/gnome/Shell/Screenshot','--method','org.gnome.Shell.Screenshot.Screenshot','false','false','/tmp/surfer-native-light.png']
 result=subprocess.run(shot,capture_output=True,text=True)
 print('Screenshot:',result.stdout,result.stderr,flush=True)

 def click(x,y):
  evaluate(f"global.surferPointer.notify_absolute_motion(imports.gi.GLib.get_monotonic_time(), {x}, {y})")
  time.sleep(.15)
  evaluate("global.surferPointer.notify_button(imports.gi.GLib.get_monotonic_time(), 1, imports.gi.Clutter.ButtonState.PRESSED)")
  time.sleep(.1)
  evaluate("global.surferPointer.notify_button(imports.gi.GLib.get_monotonic_time(), 1, imports.gi.Clutter.ButtonState.RELEASED)")
  time.sleep(.3)
 click(237,281)
 evaluate("global.surferPointer.notify_absolute_motion(imports.gi.GLib.get_monotonic_time(),1500,950)")
 time.sleep(.3)
 print('Expanded:',subprocess.run(shot,capture_output=True,text=True).stdout,flush=True)
 click(1050,190)
 shot[-1]='/tmp/surfer-theme-menu.png'
 print('Menu:',subprocess.run(shot,capture_output=True,text=True).stdout,flush=True)


 click(1030,335)
 time.sleep(.5)
 shot[-1]='/tmp/surfer-native-dracula.png'
 print('Dracula:',subprocess.run(shot,capture_output=True,text=True).stdout,flush=True)
 click(1213,190)
 time.sleep(.5)
 assert '[true,true]' in evaluate("JSON.stringify([global.surferWindow.maximized_horizontally,global.surferWindow.maximized_vertically])")
 shot[-1]='/tmp/surfer-native-maximized.png'
 print('Maximized:',subprocess.run(shot,capture_output=True,text=True).stdout,flush=True)
 evaluate("global.surferWindow.unmaximize(3); global.surferWindow.move_resize_frame(false,180,170,1100,700)")
 time.sleep(.5)
 evaluate("global.surferPointer.notify_absolute_motion(imports.gi.GLib.get_monotonic_time(),1278,500)")
 time.sleep(.1)
 evaluate("global.surferPointer.notify_button(imports.gi.GLib.get_monotonic_time(),1,imports.gi.Clutter.ButtonState.PRESSED)")
 time.sleep(.1)
 evaluate("global.surferPointer.notify_absolute_motion(imports.gi.GLib.get_monotonic_time(),1338,500)")
 time.sleep(.1)
 evaluate("global.surferPointer.notify_button(imports.gi.GLib.get_monotonic_time(),1,imports.gi.Clutter.ButtonState.RELEASED)")
 time.sleep(.3)
 assert '[1160,700]' in evaluate("JSON.stringify([global.surferWindow.get_frame_rect().width,global.surferWindow.get_frame_rect().height])")
 evaluate("global.surferWindow.move_resize_frame(false,180,170,520,320)")
 time.sleep(.5)
 shot[-1]='/tmp/surfer-native-narrow.png'
 print('Narrow:',subprocess.run(shot,capture_output=True,text=True).stdout,flush=True)
 assert app.poll() is None, 'Surfer exited during native interactions'
 print('App alive:', app.poll() is None,flush=True)

finally:
 if app is not None:app.terminate();app.wait()
 shell.terminate();shell.wait(timeout=10)
'''
result=subprocess.run(['dbus-run-session','--','python3','-c',worker],env=env,capture_output=True,text=True,timeout=50)
print(result.stdout);print(result.stderr[-1800:]);print('Logs: /tmp/surfer-native-app.log /tmp/surfer-compositor.log')

raise SystemExit(result.returncode)
