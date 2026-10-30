-- Waydesk's Hyprland configuration: the account's own, then one change.
--
-- Hyprland is started with `--config /usr/lib/waydesk/hyprland.lua`. This loads the account's configuration exactly as Hyprland would have
-- (Omarchy's, through ~/.config/hypr/hyprland.lua), and afterwards puts /usr/lib/waydesk/bin back in front of PATH for everything Hyprland starts:
-- Omarchy's own configuration puts $OMARCHY_PATH/bin first, which hid the stand-ins there (logout, reboot, shutdown, the browser policy helper).
local home = os.getenv("HOME") or ""
local config_dir = home .. "/.config/hypr"
local user_config = config_dir .. "/hyprland.lua"

-- Hyprland finds the `require`s of a Lua configuration next to the main configuration file. The main file is this one, in /usr/lib/waydesk, so
-- without this a configuration with require("monitors") or require("conf.keybinding") (ML4W, and most others) fails at its first require.
package.path = config_dir .. "/?.lua;" .. config_dir .. "/?/init.lua;" .. package.path

local runtime = os.getenv("XDG_RUNTIME_DIR") or "/tmp"
local f = io.open(user_config, "r")
if f then
  f:close()
  -- A configuration that fails must not stop the rest of this file: the session still needs its PATH, its output rule and the marker below, or the
  -- client would see nothing at all. The error is left in a file for the Waydesk session to log.
  local ok, err = pcall(dofile, user_config)
  local ef = io.open(runtime .. "/waydesk-config-error.txt", "w")
  if ef then
    ef:write(ok and "" or tostring(err))
    ef:close()
  end
end

local ours = "/usr/lib/waydesk/bin"
local omarchy_bin = (os.getenv("OMARCHY_PATH") or "/usr/share/omarchy") .. "/bin"
local seen, parts = { [ours] = true, [omarchy_bin] = true }, { ours, omarchy_bin }
for entry in (os.getenv("PATH") or "/usr/local/bin:/usr/bin"):gmatch("[^:]+") do
  if not seen[entry] then
    seen[entry] = true
    table.insert(parts, entry)
  end
end
hl.env("PATH", table.concat(parts, ":"))

-- The size and scale of the session's output. Omarchy's own configuration has a rule for every monitor (output = "", mode = "preferred"), which a
-- (re)load applies to the session's headless output as well and so puts it back to 1920x1080, whatever the client asked for. The Waydesk session writes the
-- rule it wants for its output to this file; loading it after the account's configuration makes that rule the one that stays, through reloads too.
local rule = runtime .. "/waydesk-monitor.lua"
local rf = io.open(rule, "r")
if rf then
  rf:close()
  pcall(dofile, rule)
end

-- The configuration is loaded. Hyprland answers hyprctl before it gets here; the session waits for this file before it makes its output, or the
-- rule of its output could be overwritten by the load that was still running (seen with eight sessions starting at once).
local marker = io.open(runtime .. "/waydesk-hyprland-ready", "w")
if marker then
  marker:write("1")
  marker:close()
end
