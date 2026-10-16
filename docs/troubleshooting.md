# Troubleshooting

Start with metadata:

```sh
systemctl --user status jieli-camera-bridge.service
journalctl --user -u jieli-camera-bridge.service -n 60 --no-pager
cat "$XDG_RUNTIME_DIR/jieli-camera-bridge.json"
lsusb -d 1224:2a25
```

Logs/state can include local paths, USB addresses and PIDs. Redact before
sharing. Do not upload camera frames or full USB payload captures.

| Symptom | Check |
| --- | --- |
| Shared camera exists but shows neutral status | `error`, `recovery`, `last_reset`, `sleep_monitor_error` in state JSON |
| `helper-unavailable` | Correct `--user` installed, root-owned helper exists, exact sudo rule passes `visudo` |
| `exhausted` | Device still connected; reconnect once or restart after resolving the reported cause |
| Teardown failed/hung | Capture will not reopen over an unclosed driver; preserve error metadata and investigate kernel/device |
| Capture never starts | Stable by-id node, MJPEG720p30 support, session device permissions and logind delay inhibitor available |
| Source disappears entirely | Service/publisher crash or PipeWire restart; inspect journal, plugin path and library versions |
| Publisher stalls after app switch | Confirm child loaded the private patched plugin; distribution plugin is insufficient for the reproduced 1.6.8 path |
| No V4L2 device | Installed/signed DKMS module, headers, `/dev/video10` conflict, correct card label |
| Loopback visible but no frames | Publisher reports V4L2 output, desktop user can write the node, output uses mmap |
| Duplicate Jieli in PipeWire | User WirePlumber 0.5 rule installed and policy restarted; distinguish raw/loopback/native source |
| Browser cannot use camera | Camera permission, actual backend and optional V4L2 support; test a real WebRTC stream |

To request 20 seconds of publisher debug logging, read the publisher PID from
the runtime PID file and send that process `SIGUSR1`. Check the PID belongs
to this publisher first. Do not enable global verbose GStreamer logs, which
can reveal unrelated applications' metadata.

If you create a synthetic source with `--test-source`, stop it when done.
Its name defaults to `jieli_validation` and it must not be left as an extra
camera in the desktop. The automated unit/C tests do not publish such a node.

Stop the service to intentionally stop continuous physical capture. This can
trigger the device's known next-open fault; automatic recovery or reconnect
may be needed on the next start.
