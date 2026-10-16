# Contributing

Start with the [diagnosis](docs/diagnosis.md) and [validation](docs/validation.md).
Run mocked tests before proposing changes. Hardware tests are explicit opt-in;
never automatically reset devices, load modules or suspend a machine in tests.

For a new webcam model, provide a separate reviewed device profile and explain
why its failure matches. Changing a USB ID alone is insufficient: capture
format, supported frame intervals, device discovery and WirePlumber matching
must agree. Root recovery requires its own strict identity verification and
must never accept arbitrary USB paths or shell commands from the user service.

For a new PipeWire version, compare its dequeue and GStreamer acquire paths,
port the patch, update the pinned source checksum and validate original vs
patched behavior. Do not bypass the version gate to claim support.

Include a concise reproduction and results: continuous FPS, multiple opens,
camera switches, reconnect, suspend and hibernate where supported. Mark
untested scenarios explicitly. Avoid publishing images, USB payloads, full
home paths, hostnames, emails, device serials or credentials.

Keep the physical capture and publisher lifecycles independent. Preserve
owned immutable frame bytes, bounded queues, reset serialization and the
successful-reset barrier before reopening capture.
