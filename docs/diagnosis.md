# Diagnosis and design

## Evidence from the reference camera

The Jieli `1224:2a25` worked in Windows but failed on Linux after a stream was
stopped and reopened. GNOME Snapshot, GStreamer and direct V4L2 all exposed
the failure. The following observations locate it below application decoding:

| Condition | Observation |
| --- | --- |
| USB reset, first 720p30 MJPEG stream | 60 complete structurally valid 1280×720 JPEGs |
| Close and reopen the same format | No complete JPEG, no new JPEG SOI; no V4L2 frames |
| Raw binary usbmon capture | No lost capture packets in the compared sessions |
| UVC negotiation | Successful PROBE/COMMIT and identical 30 FPS stream settings |
| Failed session's UVC stream | Fixed FID and only one EOF; incomplete video payload |
| USB reset after failure | First-stream behavior restored |

URB buffer capacity was not treated as actual video bytes. Control messages
were classified by interface/recipient: composite-device audio controls are
not UVC video PROBE/COMMIT requests. Raw marker counts alone were insufficient;
the binary trace reconstruction validated JPEG syntax and dimensions.

The evidence supports a device/USB streaming state failure after stop/start.
It does not identify a particular firmware instruction or prove that Linux
alone is responsible. A decoder cannot reconstruct frames the device did not
deliver. The precise lower-level firmware/driver interaction remains unknown.

The known `UVC_QUIRK_WAKE_AUTOSUSPEND` and `UVC_QUIRK_STREAM_NO_FID` experiments
did not establish a working solution here; the latter also damaged first-stream
delivery. No global UVC quirk override is installed. A module parameter value
of `4294967295` is the driver's `-1` sentinel for its device table, not all
quirks enabled simultaneously.

Related history:

- [Linux media report, October 2020](https://www.spinics.net/lists/linux-media/msg180041.html):
  same USB ID, camera works initially and later needs reconnection.
- [Jieli UVC probe quirk change](https://github.com/raspberrypi/linux/commit/13a3747d6533):
  related device history, not evidence that a quirk fixes the reference failure.

## Independent publisher freeze

Persistent capture solved physical stop/start but initially exposed another
freeze in the GStreamer PipeWire publisher. Inspecting PipeWire 1.6.8 showed:

1. A busy output buffer is returned to the end of the dequeue queue, and
   `pw_stream_dequeue_buffer()` returns `EBUSY`, even if another slot is free.
2. GStreamer's pool immediately waits on its condition variable.
3. A driving publisher can need to render another buffer to trigger the next
   process notification. A consumer may also release a retained shared buffer
   without that condition notification.
4. Empty dequeue queues can return `EPIPE`; reliable-output IO can return
   `EAGAIN`. Here these are temporary availability states, not proof that the
   PipeWire transport has disconnected.

The patch scans at most `max(n_buffers, 1)` slots on `EBUSY`. For `EBUSY`,
`EPIPE` and `EAGAIN`, it waits with a bounded retry: 1 ms for an active pool
with buffers, 10 ms for an empty or paused pool. Other errors retain the
upstream condition wait. Flushing and nonblocking/paused exits remain intact.
It never overwrites a consumer's retained buffer; retaining all buffers still
causes legitimate backpressure until one is released.

Source: [GStreamer pool](https://github.com/PipeWire/pipewire/blob/1.6.8/src/gst/gstpipewirepool.c),
[sink](https://github.com/PipeWire/pipewire/blob/1.6.8/src/gst/gstpipewiresink.c),
[dequeue implementation](https://github.com/PipeWire/pipewire/blob/1.6.8/src/pipewire/stream.c).
The patch is local to this project; no upstream acceptance is claimed.

## Capture and publication lifecycles

The capture process decodes MJPEG to 1280×720 I420. It copies each sample to
owned immutable `bytes`, releases the GStreamer sample and uses a queue of
one latest frame. A writer thread feeds raw bytes to a separate publisher.
The publisher uses independent bounded queues for PipeWire and optional V4L2.

In the reference Python/GI stack, `Gst.Buffer.fill(bytearray)` took about
36.6 ms per frame, versus about 0.15 ms with `bytes`. Preserve the immutable
bytes path when modifying this code; conversion overhead can exceed a 33 ms
frame budget. These are local microbenchmark observations, not universal timings.

The publisher remains alive even if capture is absent. After 250 ms without
real input it generates a neutral I420 status image. It does not keep the last
real frame as a reconnecting placeholder. Partial pipe reads accumulate until
a full frame is available; EOF terminates publication.

## Suspend and hibernate

Merely keeping capture open did not survive sleep: the physical capture could
fail and the publisher used to disappear from apps. The final design:

1. Subscribes to logind `PrepareForSleep`, holding a pre-acquired delay FD.
2. Invalidates queued capture samples and closes the physical pipeline in a
   serialized worker, leaving publication alive.
3. Releases the delay FD when close completes, or at the actual logind delay
   budget minus a small margin using an independent timer.
4. On resume, invokes the restricted USB helper only after teardown completes.
5. Requires a successful reset, waits for the device node to stabilize, then
   opens a fresh physical capture while keeping the same publisher.

Recovery uses at most three reset attempts per recovery budget, 8/16/32-second
backoff, a helper lock/cooldown and a successful-reset barrier. Stale callbacks
and frames are rejected by generation. Missing authorization stops retries.
A hung or failed teardown prevents reset/reopen; the sleep delay still expires.
An ioctl blocked uninterruptibly in the kernel cannot be given a hard userspace
deadline, so the bridge remains in a safe unavailable state rather than opening
over it. See [systemd inhibitor documentation](https://systemd.io/INHIBITOR_LOCKS/).

Both real suspend and hibernate were validated. Clean reboot was deferred;
there is no claim that startup has passed that separate check.
