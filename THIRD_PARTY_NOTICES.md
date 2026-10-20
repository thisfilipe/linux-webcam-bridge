# Third-party notices

## PipeWire

`patches/gstpipewirepool-busy-slot.patch` modifies `src/gst/gstpipewirepool.c`
from [PipeWire 1.6.8](https://github.com/PipeWire/pipewire/tree/1.6.8).
The build downloads the upstream source without redistributing a bundled
source tree or a precompiled plugin. Keep upstream notices with derived
source/binaries.

The original file carries:

```text
SPDX-FileCopyrightText: Copyright © 2018 Wim Taymans
SPDX-License-Identifier: MIT
```

The upstream `COPYING` notice is reproduced below:

Copyright © 2018 Wim Taymans

Permission is hereby granted, free of charge, to any person obtaining a
copy of this software and associated documentation files (the "Software"),
to deal in the Software without restriction, including without limitation
the rights to use, copy, modify, merge, publish, distribute, sublicense,
and/or sell copies of the Software, and to permit persons to whom the
Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice (including the next
paragraph) shall be included in all copies or substantial portions of the
Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL
THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.

## Runtime and optional dependencies

GStreamer, GLib, PipeWire, WirePlumber, systemd/logind, sudo, Rust crates and
v4l2loopback are separate projects under their respective licenses.
In particular, the optional v4l2loopback kernel module is distributed through
the user's distribution; no kernel module binary or source is bundled here.
