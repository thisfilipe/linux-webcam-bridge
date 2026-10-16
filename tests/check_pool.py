#!/usr/bin/python3
"""Compile the actual acquire_buffer function with deterministic fake I/O.

No PipeWire connection, GStreamer pipeline, camera or service is started.
The unpatched source must fail the retained-slot regression; the patch must
pass bounded-scan, empty-pool, DONTWAIT, paused and flushing checks.
"""
from pathlib import Path
import argparse
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--build-dir', type=Path, default=Path(__file__).resolve().parents[1] / 'build/plugin')
args = parser.parse_args()
root = args.build_dir.resolve()
prelude = r'''
#include <errno.h>
#include <stdio.h>
#include <string.h>
typedef unsigned int guint;
typedef int gboolean;
typedef int GstFlowReturn;
typedef int GstBuffer;
typedef struct { guint flags; } GstBufferPoolAcquireParams;
typedef struct { guint n_buffers; int flushing, paused, cond, stream; } GstBufferPool;
typedef GstBufferPool GstPipeWirePool;
typedef struct { void *pwstream; } GstPipeWireStream;
typedef struct { int queued; GstBuffer *buf; } GstPipeWirePoolData;
struct pw_buffer { void *user_data; };
enum { GST_FLOW_ERROR=-5, GST_FLOW_OK=0, GST_FLOW_FLUSHING=-2,
       GST_FLOW_CUSTOM_ERROR_1=-100, GST_FLOW_EOS=-3 };
enum { GST_BUFFER_POOL_ACQUIRE_FLAG_DONTWAIT=1,
       GST_BUFFER_POOL_ACQUIRE_FLAG_LAST=2 };
#define g_autoptr(T) T *
#define GST_PIPEWIRE_POOL(p) (p)
#define G_UNLIKELY(v) (v)
#define GST_OBJECT_LOCK(p) ((void)0)
#define GST_OBJECT_UNLOCK(p) ((void)0)
#define GST_OBJECT_GET_LOCK(p) (p)
#define GST_BUFFER_POOL_IS_FLUSHING(p) ((p)->flushing)
#define GST_LOG_OBJECT(...) ((void)0)
#define GST_DEBUG_OBJECT(...) debug_object(__VA_ARGS__)
#define GST_WARNING_OBJECT(...) ((void)0)
#define TRUE 1
#define FALSE 0
#define MAX(a,b) ((a) > (b) ? (a) : (b))
#define G_TIME_SPAN_MILLISECOND 1000
#define g_get_monotonic_time() 1000000LL
static GstPipeWireStream stream;
static GstBuffer payload;
static GstPipeWirePoolData data;
static struct pw_buffer available;
static int responses[16], response_count, calls, waits, timed_waits, silent_release;
static long long last_deadline;
static void debug_object(void *unused, const char *format, ...) { }
static void *g_weak_ref_get(int *unused) { return &stream; }
static struct pw_buffer *pw_stream_dequeue_buffer(void *unused) {
  int response = calls < response_count ? responses[calls] : -EPIPE;
  ++calls;
  if (response > 0) return &available;
  errno = -response;
  return NULL;
}
static void g_cond_wait(int *unused, GstBufferPool *pool) {
  ++waits;
  pool->flushing = 1;
}
static int g_cond_wait_until(int *unused, GstBufferPool *pool, long long deadline) {
  ++waits;
  ++timed_waits;
  last_deadline=deadline;
  if (deadline != 1001000LL && deadline != 1010000LL) {
    fputs("Invalid retry deadline\n", stderr);
    pool->flushing=1;
    return 0;
  }
  if (silent_release) {
    /* A retained shared slot becomes available, without a process signal. */
    responses[calls]=1;
    response_count=calls+1;
  } else {
    pool->flushing=1;
  }
  return 0; /* Timeout, not a condition notification. */
}
static void setup(int a, int b, int c) {
  responses[0]=a; responses[1]=b; responses[2]=c;
  response_count=3; calls=waits=timed_waits=silent_release=0;
  last_deadline=0;
  data.queued=1; data.buf=&payload; available.user_data=&data;
}
#define CHECK(v) do { if (!(v)) { printf("FAIL line %d: %s\n", __LINE__, #v); return 1; } } while (0)
'''
suite = r'''
int main(void) {
  GstBuffer *out=NULL;
  GstBufferPool pool={.n_buffers=3};
  GstBufferPoolAcquireParams params={0};
  setup(-EBUSY,1,1);
  CHECK(acquire_buffer(&pool,&out,NULL)==GST_FLOW_OK);
  CHECK(calls==2 && waits==0 && out==&payload && data.queued==0);
  setup(-EBUSY,-EBUSY,-EBUSY); pool.flushing=0;
  CHECK(acquire_buffer(&pool,&out,NULL)==GST_FLOW_FLUSHING);
  CHECK(calls==3 && waits==1);
  setup(-EPIPE,1,1); pool.flushing=0;
  CHECK(acquire_buffer(&pool,&out,NULL)==GST_FLOW_FLUSHING);
  CHECK(calls==1 && waits==1);
  setup(-EBUSY,1,1); pool.flushing=0;
  params.flags=GST_BUFFER_POOL_ACQUIRE_FLAG_DONTWAIT;
  CHECK(acquire_buffer(&pool,&out,&params)==GST_FLOW_OK);
  CHECK(calls==2 && waits==0);
  setup(-EBUSY,-EBUSY,-EBUSY); pool.flushing=0;
  CHECK(acquire_buffer(&pool,&out,&params)==GST_FLOW_EOS);
  CHECK(calls==3 && waits==0);
  setup(-EBUSY,-EBUSY,-EBUSY); pool.flushing=0; pool.paused=1;
  params.flags=GST_BUFFER_POOL_ACQUIRE_FLAG_LAST;
  CHECK(acquire_buffer(&pool,&out,&params)==GST_FLOW_CUSTOM_ERROR_1);
  CHECK(calls==3 && waits==0);
  setup(1,1,1); pool.flushing=1;
  CHECK(acquire_buffer(&pool,&out,NULL)==GST_FLOW_FLUSHING);
  CHECK(calls==0 && waits==0);
  setup(-EBUSY,-EBUSY,-EBUSY); pool.flushing=0; pool.n_buffers=0;
  CHECK(acquire_buffer(&pool,&out,NULL)==GST_FLOW_FLUSHING);
  CHECK(calls==1 && waits==1);
  setup(-EBUSY,-EBUSY,-EBUSY); pool.flushing=0; pool.n_buffers=3; pool.paused=0;
  silent_release=1;
  CHECK(acquire_buffer(&pool,&out,NULL)==GST_FLOW_OK);
  CHECK(calls==4 && waits==1 && timed_waits==1);
  setup(-EPIPE,-EPIPE,-EPIPE); pool.flushing=0;
  silent_release=1;
  CHECK(acquire_buffer(&pool,&out,NULL)==GST_FLOW_OK);
  CHECK(calls==2 && waits==1 && timed_waits==1);
  setup(-EAGAIN,-EAGAIN,-EAGAIN); pool.flushing=0;
  silent_release=1;
  CHECK(acquire_buffer(&pool,&out,NULL)==GST_FLOW_OK);
  CHECK(calls==2 && waits==1 && timed_waits==1);
  setup(-EPIPE,-EPIPE,-EPIPE); pool.flushing=0;
  params.flags=GST_BUFFER_POOL_ACQUIRE_FLAG_DONTWAIT;
  CHECK(acquire_buffer(&pool,&out,&params)==GST_FLOW_EOS);
  CHECK(calls==1 && waits==0);
  setup(-EPIPE,-EPIPE,-EPIPE); pool.flushing=0; pool.paused=1;
  params.flags=GST_BUFFER_POOL_ACQUIRE_FLAG_LAST;
  CHECK(acquire_buffer(&pool,&out,&params)==GST_FLOW_CUSTOM_ERROR_1);
  CHECK(calls==1 && waits==0);
  setup(-EINVAL,-EINVAL,-EINVAL); pool.flushing=0; pool.paused=0;
  CHECK(acquire_buffer(&pool,&out,NULL)==GST_FLOW_FLUSHING);
  CHECK(calls==1 && waits==1 && timed_waits==0);
  setup(-EPIPE,-EPIPE,-EPIPE); pool.flushing=0; pool.paused=0;
  silent_release=1;
  CHECK(acquire_buffer(&pool,&out,NULL)==GST_FLOW_OK);
  CHECK(calls==2 && waits==1 && timed_waits==1 && last_deadline==1001000LL);
  setup(-EPIPE,-EPIPE,-EPIPE); pool.flushing=0; pool.paused=1;
  CHECK(acquire_buffer(&pool,&out,NULL)==GST_FLOW_FLUSHING);
  CHECK(calls==1 && waits==1 && timed_waits==1 && last_deadline==1010000LL);
  setup(-EPIPE,-EPIPE,-EPIPE); pool.flushing=0; pool.paused=0; pool.n_buffers=0;
  CHECK(acquire_buffer(&pool,&out,NULL)==GST_FLOW_FLUSHING);
  CHECK(calls==1 && waits==1 && timed_waits==1 && last_deadline==1010000LL);
  puts("17 deterministic acquire_buffer scenarios passed");
  return 0;
}
'''

with tempfile.TemporaryDirectory(prefix='pool-tests-') as directory:
    for label, path, expected in (
        ("baseline", root / "gstpipewirepool.c.original", False),
        ("patched", root / "pipewire-1.6.8/src/gst/gstpipewirepool.c", True),
    ):
        text = path.read_text()
        start = text.index("static GstFlowReturn\nacquire_buffer")
        end = text.index("static const gchar **", start)
        generated = Path(directory) / f"test-pool-{label}.c"
        executable = generated.with_suffix('')
        generated.write_text(prelude + text[start:end] + suite)
        subprocess.run(["cc", "-std=gnu11", "-Wall", "-Wextra", "-Werror",
                        "-Wno-unused-parameter", "-Wno-unused-function",
                        str(generated), "-o", str(executable)], check=True)
        result = subprocess.run([str(executable)], text=True, capture_output=True)
        if (result.returncode == 0) != expected:
            raise SystemExit(f"Unexpected {label} result: {result.stdout} {result.stderr}")
        print(f"{label}: {result.stdout.strip()}")
