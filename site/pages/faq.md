title: Questions and answers
nav: FAQ
seo_title: Waydesk FAQ: GPU, Sway, sessions, apps, capacity, security
description: Short answers: whether a GPU is needed, why Sway, what happens when you disconnect, which apps run, how many users fit, and who makes Waydesk.
---
Short answers, with a link to the page that has the rest.

## Do I need a GPU? {bar: notes}
No. Without one, everything is drawn and encoded on the CPU. With one (Intel, NVIDIA, AMD should work) the desktop is drawn on it and the video is encoded by the chip, which leaves the CPU to your programs. [The GPU page](/docs/gpu/) has the numbers.

## What happens when I close the RDP window? {bar: notes}
Nothing: the session is a systemd unit of the account and keeps running, so reconnecting gives you the same windows. A second login from the same account takes the running session over.

## Why Sway? {bar: notes}
Each session needs a desktop that can run headless, one per account, and be captured and scripted. Sway is a small Wayland compositor that does that, and the image sent over RDP is captured from it. Windows are floating with title bars, so it feels like a desktop, not a tiling window manager.

## Which applications run? {bar: notes}
Wayland and X11 applications, from the distribution or from Flathub. X11 programs run through XWayland, one server per account. Checked: Chromium, VS Code, LibreOffice, Obsidian, Slack, GNOME Text Editor, GIMP, Zed, Blender, Kdenlive, darktable. `waydesk-setup` offers browsers, office, developer and creative applications.

## How many users fit on one server? {bar: notes}
On an 8 GB server 7 to 8 users fit with Chromium, Calc and VS Code open and left idle; people who are working use more, about 1.6 GB each on the 23 GB server we measured with active users. Memory is the limit, and a new session is refused rather than slowing everyone down. See [Limits and status](/docs/limits/).

## Is it safe to put on the internet? {bar: notes}
Not with the port published: the login is a password with no second factor. Use an SSH tunnel or a VPN, which are both a few commands. [VPN and securing access](/docs/vpn/) says what protects the login.

## Which RDP clients work? {bar: notes}
The Windows App on macOS, mstsc on Windows 11 and FreeRDP have been tried. The list, with what did not work, is on [Limits and status](/docs/limits/).

## What does it cost, and who makes it? {bar: notes}
It is free software, written and maintained by [StackCare](https://stackcare.pt/en), which also offers commercial support and setup. The licence is the AGPL, explained next.

## What licence is it under, and what does it mean for me? {bar: notes}
Waydesk is under the GNU Affero General Public License, version 3 or later. You may use it for any purpose, free of charge, including at a company and for paid work of your own, and you may charge for installing, hosting or supporting it. What the licence asks: if you give others a modified version, or let them use a modified version over a network, they must be able to get its source under the same licence. This is a summary, not legal advice: the [licence text](https://github.com/zhoorta/waydesk/blob/main/LICENSE) decides. If the AGPL does not fit your case, write to [StackCare](https://stackcare.pt/en).

## Can I see how it is built? {bar: notes}
Yes: [Architecture](/docs/architecture/) draws the three parts, [Measurements](/docs/measurements/) shows how each figure was taken, and the source is on [GitHub](https://github.com/zhoorta/waydesk).
