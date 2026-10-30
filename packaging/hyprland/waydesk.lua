-- Waydesk's Hyprland configuration: the account's own, then one change.
--
-- Hyprland is started with `--config /usr/lib/waydesk/hyprland.lua`. This loads the account's configuration exactly as Hyprland would have
-- (Omarchy's, through ~/.config/hypr/hyprland.lua), and afterwards puts /usr/lib/waydesk/bin back in front of PATH for everything Hyprland starts:
-- Omarchy's own configuration puts $OMARCHY_PATH/bin first, which hid the stand-ins there (logout, reboot, shutdown, the browser policy helper).
local home = os.getenv("HOME") or ""
local user_config = home .. "/.config/hypr/hyprland.lua"

local f = io.open(user_config, "r")
if f then
  f:close()
  dofile(user_config)
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
