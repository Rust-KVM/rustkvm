#define _POSIX_C_SOURCE 200809L
#define _DEFAULT_SOURCE
#include <unistd.h>
#include <time.h>
#include <string.h>
#include <stdbool.h>
#include <fcntl.h>
#include <linux/videodev2.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/select.h>
#include <signal.h>
#include <stdint.h>

#include <rk_type.h>
#include <rk_mpi_venc.h>
#include <rk_mpi_mb.h>
#include <rk_mpi_sys.h>

// FFI callback provided by Rust to receive one encoded H.264 frame
extern void rustkvm_on_h264_frame(const uint8_t* data, size_t len, uint64_t pts_us);
extern void rustkvm_on_video_state_changed(int ready, uint16_t width, uint16_t height, double fps, const char *error);

typedef struct {
    bool ready;
    const char *error;
    u_int16_t width;
    u_int16_t height;
    double frame_per_second;
} video_state_t;

static video_state_t video_state = {false, NULL, 0, 0, 0.0};

#ifndef RK_SUCCESS
#define RK_SUCCESS 0
#endif

#ifndef RK_NULL
#define RK_NULL ((void*)0)
#endif
#ifndef RK_TRUE
#define RK_TRUE 1
#endif
#ifndef RK_ERR_VENC_BUF_EMPTY
#define RK_ERR_VENC_BUF_EMPTY 0x40408003
#endif
#define VIDEO_DEV "/dev/video0"
#define ALIGN(x, a) (((x) + (a)-1) & ~((a)-1))
#define ALIGN_2(x) ALIGN(x, 2)
#define VENC_CHANNEL 0
#define INPUT_BUFFER_COUNT 8  // Match yavta default
#define SLEEP_100MS 100000
#define SLEEP_1S 1000000
#define BASE_BITRATE_HIGH 2000
#define BASE_BITRATE_LOW 512
#define MIN_BITRATE 100
#define REF_WIDTH 1920
#define REF_HEIGHT 1080
static bool should_exit = false;
static bool streaming_flag = false;
static uint32_t detected_width = 0;   // Auto-detect from device
static uint32_t detected_height = 0;  // Auto-detect from device
static float quality_factor = 1.0f;
static pthread_t *streaming_thread = NULL;
// V4L2 hardware timestamp tracking (yavta best practice)
static struct timeval first_frame_timestamp = {0};
static struct timeval last_frame_timestamp = {0};
static bool first_frame_captured = false;
static void video_start_streaming(void);
static void video_stop_streaming(void);

static void init_v4l2_buffer(struct v4l2_buffer *buf, enum v4l2_buf_type type, int index, struct v4l2_plane *plane, int num_planes, enum v4l2_memory memtype)
{
    memset(buf, 0, sizeof(*buf));
    buf->type = type;
    buf->memory = memtype;
    buf->index = index;
    buf->m.planes = plane;
    buf->length = num_planes;  // Use actual number of planes
}

static pthread_t* create_thread(void *(*thread_func)(void *), void *arg, const char *error_msg)
{
    pthread_t *thread = malloc(sizeof(pthread_t));
    if (!thread) {
        printf("Failed to allocate memory for thread\n");
        return NULL;
    }

    if (pthread_create(thread, NULL, thread_func, arg) != 0) {
        printf("%s\n", error_msg);
        free(thread);
        return NULL;
    }

    return thread;
}

static int handle_video_error(int fd, int *retry_count, const int max_retries)
{
    close(fd);
    (*retry_count)++;
    if (*retry_count >= max_retries) {
        printf("Max retries reached for device setup, giving up\n");
        return -1; // break
    }
    usleep(SLEEP_100MS);
    return 0; // continue
}

static pthread_t *venc_read_thread = NULL;
static volatile bool venc_running = false;

static MB_POOL memPool = MB_INVALID_POOLID;
// static FILE *h264_output_file = NULL;
struct video_buffer {
    struct v4l2_plane plane_buffer;
    void *mapped_addr;
    size_t length;
    uint32_t bytesperline;  // Stride from driver
    uint32_t sizeimage;     // Actual image size

    MB_BLK mb_blk;
};

static inline int32_t calculate_bitrate(float bitrate_factor, uint32_t width, uint32_t height)
{
    const double scale_factor = ((double)width * height) / (REF_WIDTH * REF_HEIGHT);
    const int32_t base_bitrate = BASE_BITRATE_LOW + (int32_t)((BASE_BITRATE_HIGH - BASE_BITRATE_LOW) * bitrate_factor);
    const int32_t bitrate = (int32_t)(base_bitrate * scale_factor);
    return (bitrate < MIN_BITRATE) ? MIN_BITRATE : bitrate;
}


static void populate_venc_attr(VENC_CHN_ATTR_S *attr, uint32_t bitrate,
                               uint32_t max_bitrate, uint32_t width, uint32_t height,
                               uint32_t stride, uint32_t fps)
{
    memset(attr, 0, sizeof(VENC_CHN_ATTR_S));

    // Rate control: VBR mode
    attr->stRcAttr.enRcMode = VENC_RC_MODE_H264VBR;
    attr->stRcAttr.stH264Vbr.u32BitRate = bitrate;
    attr->stRcAttr.stH264Vbr.u32MaxBitRate = max_bitrate;
    attr->stRcAttr.stH264Vbr.u32Gop = fps;  // GOP = frame rate (1 second)
    attr->stRcAttr.stH264Vbr.u32SrcFrameRateNum = fps;  // Source FPS
    attr->stRcAttr.stH264Vbr.u32SrcFrameRateDen = 1;
    attr->stRcAttr.stH264Vbr.fr32DstFrameRateNum = fps;  // Output FPS
    attr->stRcAttr.stH264Vbr.fr32DstFrameRateDen = 1;

    // Video encoder attributes
    attr->stVencAttr.enType = RK_VIDEO_ID_AVC;
    attr->stVencAttr.enPixelFormat = RK_FMT_YUV420SP;  // NV12
    attr->stVencAttr.u32Profile = H264E_PROFILE_HIGH;
    attr->stVencAttr.u32PicWidth = width;
    attr->stVencAttr.u32PicHeight = height;
    attr->stVencAttr.u32VirWidth = stride;  // Use actual stride from driver
    attr->stVencAttr.u32VirHeight = ALIGN_2(height);
    attr->stVencAttr.u32StreamBufCnt = 4;  // Reduce to avoid latency
    attr->stVencAttr.u32BufSize = stride * height * 3 / 2;  // Use stride for buffer size
    attr->stVencAttr.enMirror = MIRROR_NONE;
}
static void *venc_read_stream(void *arg)
{
    (void)arg;
    uint32_t frame_count = 0;
    VENC_STREAM_S frame = {0};

    frame.pstPack = malloc(sizeof(VENC_PACK_S));
    if (!frame.pstPack) {
        printf("ERROR: Failed to allocate VENC_PACK_S\n");
        return NULL;
    }
    memset(frame.pstPack, 0, sizeof(VENC_PACK_S));

    while (venc_running) {
        int32_t ret = RK_MPI_VENC_GetStream(VENC_CHANNEL, &frame, 200);

        if (ret == RK_SUCCESS) {
            void *data = RK_MPI_MB_Handle2VirAddr(frame.pstPack->pMbBlk);

            // Print status every 60 frames (1 second at 60fps)
            if (++frame_count % 60 == 0) {
                printf("Encoded %u frames, size: %u bytes, PTS: %llu us\n",
                       frame_count, frame.pstPack->u32Len, frame.pstPack->u64PTS);
            }

            // Deliver frame to Rust via FFI (one frame per call)
            rustkvm_on_h264_frame((const uint8_t*)data, frame.pstPack->u32Len, frame.pstPack->u64PTS);

            ret = RK_MPI_VENC_ReleaseStream(VENC_CHANNEL, &frame);
            if (ret != RK_SUCCESS) {
                printf("ERROR: Failed to release stream: 0x%x\n", ret);
            }
        } else if (ret != RK_ERR_VENC_BUF_EMPTY) {
            printf("ERROR: GetStream failed: 0x%x\n", ret);
            break;
        }
    }

    printf("Encoder stopped, %u frames sent to Rust\n", frame_count);

    free(frame.pstPack);
    return NULL;
}
static int32_t venc_start(int32_t bitrate, int32_t max_bitrate,
                          int32_t width, int32_t height, uint32_t stride, uint32_t fps)
{
    VENC_CHN_ATTR_S attr;
    populate_venc_attr(&attr, bitrate, max_bitrate, width, height, stride, fps);

    int32_t ret = RK_MPI_VENC_CreateChn(VENC_CHANNEL, &attr);
    if (ret < 0) {
        printf("Failed to create encoder channel: %d\n", ret);
        return ret;
    }

    VENC_RECV_PIC_PARAM_S recv_param;
    memset(&recv_param, 0, sizeof(recv_param));
    recv_param.s32RecvPicNum = -1;

    ret = RK_MPI_VENC_StartRecvFrame(VENC_CHANNEL, &recv_param);
    if (ret < 0) {
        printf("Failed to start receiving frames: %d\n", ret);
        return ret;
    }

    venc_running = true;
    venc_read_thread = create_thread(venc_read_stream, NULL, "Failed to create encoder thread");
    if (!venc_read_thread) {
        return -1;
    }

    return 0;
}
static int32_t venc_stop(void)
{
    if (!venc_running) return RK_SUCCESS;

    venc_running = false;
    RK_MPI_VENC_StopRecvFrame(VENC_CHANNEL);

    if (venc_read_thread) {
        pthread_join(*venc_read_thread, NULL);
        free(venc_read_thread);
        venc_read_thread = NULL;
    }

    int32_t ret = RK_MPI_VENC_DestroyChn(VENC_CHANNEL);
    if (ret != RK_SUCCESS) {
        printf("WARNING: Failed to destroy encoder channel: 0x%x\n", ret);
    }
    return ret;
}
static int32_t init_memory_pool(void)
{
    // Try allocating memory pool with fallback sizes (NV12 format: width * height * 3/2)
    const struct {
        uint32_t width;
        uint32_t height;
        const char *name;
    } sizes[] = {
        {3840, 2160, "4K"},
        {1920, 1080, "1080p"},
        {1280, 720, "720p"}
    };

    for (size_t i = 0; i < sizeof(sizes) / sizeof(sizes[0]); i++) {
        MB_POOL_CONFIG_S cfg = {
            .u64MBSize = sizes[i].width * sizes[i].height * 3 / 2,  // NV12
            .u32MBCnt = INPUT_BUFFER_COUNT,  // Match yavta: 8 buffers
            .enAllocType = MB_ALLOC_TYPE_DMA,
            .bPreAlloc = RK_FALSE
        };

        memPool = RK_MPI_MB_CreatePool(&cfg);
        if (memPool != MB_INVALID_POOLID) {
            uint32_t total_mb = (uint32_t)(cfg.u64MBSize * cfg.u32MBCnt / 1024 / 1024);
            printf("MPP memory pool created: %s (%u buffers, %u MB total)\n",
                   sizes[i].name, cfg.u32MBCnt, total_mb);
            return RK_SUCCESS;
        }
    }

    printf("ERROR: Failed to create MPP memory pool\n");
    return -1;
}

static void *run_video_stream(void *arg)
{
    (void)arg;
    enum v4l2_buf_type type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    int retry_count = 0;
    const int max_retries = 10;
    int num_planes = 1;  // Track number of planes
    uint32_t fps = 60;    // Default frame rate, will be detected

    while (streaming_flag) {
        int video_fd = open(VIDEO_DEV, O_RDWR | O_NONBLOCK);
        if (video_fd < 0) {
            printf("Failed to open %s: %s\n", VIDEO_DEV, strerror(errno));
            usleep(SLEEP_1S);
            continue;
        }
        printf("Opened video device %s\n", VIDEO_DEV);

        struct v4l2_format fmt;
        memset(&fmt, 0, sizeof(fmt));
        fmt.type = type;

        // First, get the current format from device
        if (ioctl(video_fd, VIDIOC_G_FMT, &fmt) < 0) {
            perror("Failed to get format");
            if (handle_video_error(video_fd, &retry_count, max_retries) < 0) break;
            continue;
        }

        // Auto-detect resolution on first open
        if (detected_width == 0 || detected_height == 0) {
            detected_width = fmt.fmt.pix_mp.width;
            detected_height = fmt.fmt.pix_mp.height;
            num_planes = fmt.fmt.pix_mp.num_planes;

            // Try to get frame rate from DV timings (for HDMI sources)
            struct v4l2_dv_timings timings;
            memset(&timings, 0, sizeof(timings));
            if (ioctl(video_fd, VIDIOC_G_DV_TIMINGS, &timings) == 0) {
                // Calculate FPS from pixel clock and frame dimensions
                if (timings.type == V4L2_DV_BT_656_1120) {
                    uint64_t pixelclock = timings.bt.pixelclock;
                    uint32_t htotal = timings.bt.width + timings.bt.hfrontporch +
                                      timings.bt.hsync + timings.bt.hbackporch;
                    uint32_t vtotal = timings.bt.height + timings.bt.vfrontporch +
                                      timings.bt.vsync + timings.bt.vbackporch;
                    if (htotal > 0 && vtotal > 0) {
                        fps = (uint32_t)(pixelclock / (htotal * vtotal));
                    }
                }
            }

            printf("Detected: %ux%u %.4s %d-plane @ %u fps\n",
                   detected_width, detected_height,
                   (char*)&fmt.fmt.pix_mp.pixelformat, num_planes, fps);
        }

        // Set format - keep device's current pixel format, force progressive
        fmt.fmt.pix_mp.width = detected_width;
        fmt.fmt.pix_mp.height = detected_height;
        fmt.fmt.pix_mp.pixelformat = V4L2_PIX_FMT_NV12;  // Explicit NV12
        fmt.fmt.pix_mp.field = V4L2_FIELD_NONE;  // Force progressive (no interlacing)

        if (ioctl(video_fd, VIDIOC_S_FMT, &fmt) < 0) {
            perror("Failed to set format");
            if (handle_video_error(video_fd, &retry_count, max_retries) < 0) break;
            continue;
        }

        // Update num_planes after S_FMT in case it changed
        num_planes = fmt.fmt.pix_mp.num_planes;

        // Get actual stride and buffer size from driver
        const uint32_t stride = fmt.fmt.pix_mp.plane_fmt[0].bytesperline;
        const uint32_t sizeimage = fmt.fmt.pix_mp.plane_fmt[0].sizeimage;

        // Print actual format applied by driver
        printf("Applied format: %ux%u %.4s %d-plane, stride=%u, size=%u, field=%d\n",
               fmt.fmt.pix_mp.width, fmt.fmt.pix_mp.height,
               (char*)&fmt.fmt.pix_mp.pixelformat, num_planes, stride, sizeimage,
               fmt.fmt.pix_mp.field);

        retry_count = 0;

        // Request MMAP buffers from driver
        struct v4l2_requestbuffers req = {
            .count = INPUT_BUFFER_COUNT,
            .type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE,
            .memory = V4L2_MEMORY_MMAP
        };

        if (ioctl(video_fd, VIDIOC_REQBUFS, &req) < 0) {
            perror("ERROR: Request buffers failed");
            close(video_fd);
            continue;
        }

        // For MMAP mode: QUERYBUF then mmap
        struct video_buffer buffers[INPUT_BUFFER_COUNT];
        memset(buffers, 0, sizeof(buffers));

        for (int i = 0; i < INPUT_BUFFER_COUNT; i++) {
            struct v4l2_buffer buf;
            init_v4l2_buffer(&buf, type, i, &buffers[i].plane_buffer, num_planes, V4L2_MEMORY_MMAP);

            if (ioctl(video_fd, VIDIOC_QUERYBUF, &buf) < 0) {
                perror("Failed to query buffer");
                break;
            }

            buffers[i].length = buffers[i].plane_buffer.length;
            buffers[i].bytesperline = stride;
            buffers[i].sizeimage = sizeimage;

            // mmap the V4L2 buffer
            buffers[i].mapped_addr = mmap(NULL, buffers[i].plane_buffer.length,
                                          PROT_READ | PROT_WRITE, MAP_SHARED,
                                          video_fd, buffers[i].plane_buffer.m.mem_offset);
            if (buffers[i].mapped_addr == MAP_FAILED) {
                printf("ERROR: mmap buffer %d failed: %s\n", i, strerror(errno));
                buffers[i].mapped_addr = NULL;
                break;
            }

            // Allocate MPP buffer for encoder
            buffers[i].mb_blk = RK_MPI_MB_GetMB(memPool, buffers[i].length, RK_TRUE);
            if (!buffers[i].mb_blk) {
                printf("ERROR: MPP buffer %d allocation failed\n", i);
                break;
            }
        }
        printf("Buffers setup: %d MMAP buffers, stride=%u, size=%u\n",
               INPUT_BUFFER_COUNT, stride, sizeimage);

        // Queue all buffers to driver
        for (int i = 0; i < INPUT_BUFFER_COUNT; i++) {
            struct v4l2_buffer buf = {0};
            init_v4l2_buffer(&buf, type, i, &buffers[i].plane_buffer, num_planes, V4L2_MEMORY_MMAP);

            if (ioctl(video_fd, VIDIOC_QBUF, &buf) < 0) {
                printf("ERROR: Queue buffer %d failed: %s\n", i, strerror(errno));
                // Cleanup successfully allocated buffers
                for (int j = 0; j < i; j++) {
                    if (buffers[j].mb_blk) {
                        RK_MPI_MB_ReleaseMB(buffers[j].mb_blk);
                        buffers[j].mb_blk = NULL;
                    }
                }
                goto cleanup;
            }
        }

        // Start V4L2 streaming
        if (ioctl(video_fd, VIDIOC_STREAMON, &type) < 0) {
            perror("ERROR: Start streaming failed");
            goto cleanup;
        }

        // Start encoder with detected frame rate and actual stride
        const int32_t bitrate = calculate_bitrate(quality_factor, detected_width, detected_height);
        if (venc_start(bitrate, bitrate * 2, detected_width, detected_height, stride, fps) != 0) {
            printf("ERROR: Encoder start failed\n");
            goto cleanup;
        }

        printf("Streaming started: %ux%u @ %u fps (stride=%u), bitrate=%d Kbps\n",
               detected_width, detected_height, fps, stride, bitrate);
        report_video_format(true, NULL, (u_int16_t)detected_width, (u_int16_t)detected_height, (double)fps);

        fd_set fds;
        struct timeval tv;
        uint32_t frame_count = 0;

        while (streaming_flag) {
            FD_ZERO(&fds);
            FD_SET(video_fd, &fds);
            tv.tv_sec = 0;
            tv.tv_usec = 50000;  // Reduce timeout to 50ms for better responsiveness

            int ret = select(video_fd + 1, &fds, NULL, NULL, &tv);
            if (ret == 0) {
                continue;  // Don't break on timeout, just retry
            } else if (ret < 0) {
                if (errno == EINTR) continue;
                perror("Select failed");
                break;
            }

            struct v4l2_buffer buf = {0};
            struct v4l2_plane plane = {0};
            init_v4l2_buffer(&buf, type, 0, &plane, num_planes, V4L2_MEMORY_MMAP);

            if (ioctl(video_fd, VIDIOC_DQBUF, &buf) < 0) {
                perror("Failed to dequeue buffer");
                break;
            }

            const size_t frame_size = plane.bytesused;
            MB_BLK blk = buffers[buf.index].mb_blk;

            // Calculate PTS using V4L2 hardware timestamp (yavta best practice)
            uint64_t frame_pts_us;
            if (!first_frame_captured) {
                // First frame: record base timestamp
                first_frame_timestamp.tv_sec = buf.timestamp.tv_sec;
                first_frame_timestamp.tv_usec = buf.timestamp.tv_usec;
                last_frame_timestamp = first_frame_timestamp;
                frame_pts_us = 0;
                first_frame_captured = true;
            } else {
                // Subsequent frames: calculate relative timestamp
                uint64_t current_us = (uint64_t)buf.timestamp.tv_sec * 1000000ULL +
                                      (uint64_t)buf.timestamp.tv_usec;
                uint64_t start_us = (uint64_t)first_frame_timestamp.tv_sec * 1000000ULL +
                                    (uint64_t)first_frame_timestamp.tv_usec;
                frame_pts_us = current_us - start_us;
            }

            // Calculate actual FPS from V4L2 timestamps (yavta best practice)
            double actual_fps = 0.0;
            if (frame_count > 0) {
                double elapsed_us = (buf.timestamp.tv_sec - last_frame_timestamp.tv_sec) * 1000000.0
                                    + (buf.timestamp.tv_usec - last_frame_timestamp.tv_usec);
                actual_fps = elapsed_us ? 1000000.0 / elapsed_us : 0.0;
            }

            // Update last timestamp for next frame (yavta pattern)
            last_frame_timestamp.tv_sec = buf.timestamp.tv_sec;
            last_frame_timestamp.tv_usec = buf.timestamp.tv_usec;

            // Print every 60 frames
            if (++frame_count % 60 == 0) {
                printf("Captured %u frames (size=%zu, FPS: %.2f)\n",
                       frame_count, frame_size, actual_fps);
            }

            if (blk == RK_NULL) {
                printf("ERROR: Buffer %d has no valid MB block\n", buf.index);
            } else {
                // Copy frame data from V4L2 MMAP to MPP buffer
                void *mpp_addr = RK_MPI_MB_Handle2VirAddr(blk);
                if (mpp_addr && buffers[buf.index].mapped_addr) {
                    const size_t copy_size = (frame_size < buffers[buf.index].sizeimage) ?
                                             frame_size : buffers[buf.index].sizeimage;
                    memcpy(mpp_addr, buffers[buf.index].mapped_addr, copy_size);
                    // Ensure CPU writes are visible to VENC hardware
                    RK_MPI_SYS_MmzFlushCache(buffers[buf.index].mb_blk, RK_FALSE);
                }

                // Prepare frame for encoder
                VIDEO_FRAME_INFO_S stFrame = {0};
                stFrame.stVFrame.pMbBlk = blk;
                stFrame.stVFrame.u32Width = detected_width;
                stFrame.stVFrame.u32Height = detected_height;
                stFrame.stVFrame.u32VirWidth = buffers[buf.index].bytesperline;
                stFrame.stVFrame.u32VirHeight = ALIGN_2(detected_height);
                stFrame.stVFrame.u32TimeRef = frame_count;
                stFrame.stVFrame.u64PTS = frame_pts_us;  // Use V4L2 hardware timestamp
                stFrame.stVFrame.enPixelFormat = RK_FMT_YUV420SP;
                stFrame.stVFrame.enCompressMode = COMPRESS_MODE_NONE;

                // Send frame to encoder (MUST finish before QBUF)
                int32_t ret = RK_MPI_VENC_SendFrame(VENC_CHANNEL, &stFrame, 2000);
                if (ret != RK_SUCCESS) {
                    usleep(500);
                    ret = RK_MPI_VENC_SendFrame(VENC_CHANNEL, &stFrame, 1000);
                    if (ret != RK_SUCCESS) {
                        printf("WARNING: SendFrame failed after retry: 0x%x (skipping frame)\n", ret);
                    }
                }
            }

            // Requeue buffer AFTER encoder has taken the frame
            if (ioctl(video_fd, VIDIOC_QBUF, &buf) < 0) {
                perror("Failed to requeue buffer");
                break;
            }
        }

cleanup:
        // Stop encoder first (before releasing buffers)
        venc_stop();

        report_video_format(false, "stopped", 0, 0, 0.0);

        // Stop V4L2 streaming
        if (video_fd >= 0) {
            ioctl(video_fd, VIDIOC_STREAMOFF, &type);
        }

        // Release buffers (V4L2 MMAP and MPP)
        for (int i = 0; i < INPUT_BUFFER_COUNT; i++) {
            if (buffers[i].mapped_addr && buffers[i].mapped_addr != MAP_FAILED) {
                munmap(buffers[i].mapped_addr, buffers[i].length);
                buffers[i].mapped_addr = NULL;
            }
            if (buffers[i].mb_blk) {
                RK_MPI_MB_ReleaseMB(buffers[i].mb_blk);
                buffers[i].mb_blk = NULL;
            }
        }

        if (video_fd >= 0) {
            close(video_fd);
            video_fd = -1;
        }
        printf("Video stream stopped\n");
    }

    return NULL;
}
// Removed run_detect_format function - using device auto-detection instead
static int video_init(void)
{
    printf("Initializing video system...\n");

    if (init_memory_pool() != RK_SUCCESS) {
        printf("ERROR: Memory pool initialization failed\n");
        return -1;
    }

    printf("Video system ready\n");
    return 0;
}
static void video_start_streaming(void)
{
    if (streaming_thread) {
        printf("Video streaming already started\n");
        return;
    }

    // Reset timestamp tracking for new stream
    first_frame_captured = false;
    memset(&first_frame_timestamp, 0, sizeof(first_frame_timestamp));
    memset(&last_frame_timestamp, 0, sizeof(last_frame_timestamp));

    streaming_flag = true;
    streaming_thread = create_thread(run_video_stream, NULL, "Failed to create streaming thread");
    if (!streaming_thread) {
        streaming_flag = false;
    }
}
static void video_stop_streaming(void)
{
    if (!streaming_thread) return;

    streaming_flag = false;
    usleep(100000);  // Give thread 100ms to exit cleanly
    pthread_join(*streaming_thread, NULL);
    free(streaming_thread);
    streaming_thread = NULL;
    printf("Video streaming stopped\n");
}

static void video_shutdown(void)
{
    if (should_exit) return;
    should_exit = true;

    video_stop_streaming();

    if (memPool != MB_INVALID_POOLID) {
        RK_MPI_MB_DestroyPool(memPool);
        memPool = MB_INVALID_POOLID;
    }

    printf("Video system shutdown complete\n");
}
static void signal_handler(int sig)
{
    static volatile bool shutting_down = false;

    if (shutting_down) {
        return;  // Already shutting down
    }
    shutting_down = true;

    printf("\nReceived signal %d, shutting down...\n", sig);
    should_exit = true;
    video_shutdown();
    exit(0);
}
#ifndef RUSTKVM_AS_LIB
int main(int argc, char *argv[])
{
    printf("Video Capture & H.264 Encoder\n");

    // Setup signal handlers
    signal(SIGINT, signal_handler);
    signal(SIGTERM, signal_handler);

    // Initialize Rockchip MPP
    if (RK_MPI_SYS_Init() != RK_SUCCESS) {
        printf("ERROR: MPP system init failed\n");
        return -1;
    }
    printf("MPP system initialized\n");

    // Parse command line arguments
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--quality") == 0 && i + 1 < argc) {
            quality_factor = atof(argv[i + 1]);
            if (quality_factor < 0.0f) quality_factor = 0.0f;
            if (quality_factor > 1.0f) quality_factor = 1.0f;
            i++;
        } else if (strcmp(argv[i], "--help") == 0) {
            printf("Usage: %s [options]\n", argv[0]);
            printf("Options:\n");
            printf("  --quality <0.0-1.0>  Video quality (default: 1.0)\n");
            printf("  --help               Show this help\n");
            return 0;
        }
    }

    // Initialize video system
    if (video_init() != 0) {
        goto cleanup;
    }

    // Start capture and encoding
    video_start_streaming();

    // Main loop
    while (!should_exit) {
        usleep(SLEEP_1S);
    }

cleanup:
    video_shutdown();
    RK_MPI_SYS_Exit();
    printf("Program exited\n");
    return 0;
}
#endif // RUSTKVM_AS_LIB

// Library API for Rust control
int rustkvm_native_video_init(void)
{
    if (RK_MPI_SYS_Init() != RK_SUCCESS) {
        printf("Failed to initialize Rockchip MPP system\n");
        return -1;
    }
    if (video_init() != 0) {
        printf("Failed to initialize video system\n");
        return -1;
    }
    return 0;
}

void rustkvm_native_video_start(void)
{
    video_start_streaming();
}

void rustkvm_native_video_stop(void)
{
    video_stop_streaming();
}

void rustkvm_native_video_shutdown(void)
{
    video_shutdown();
    RK_MPI_SYS_Exit();
}

void rustkvm_native_video_set_quality(float q)
{
    quality_factor = q;
}

void report_video_format(bool ready, const char *error, u_int16_t width, u_int16_t height, double frame_per_second)
{
    video_state.ready = ready;
    video_state.error = error;
    video_state.width = width;
    video_state.height = height;
    video_state.frame_per_second = frame_per_second;

    // printf("DEBUG: Calling rustkvm_on_video_state_changed with ready=%d, width=%u, height=%u, fps=%.2f\n",
    //   ready ? 1 : 0, width, height, frame_per_second);

    rustkvm_on_video_state_changed(ready ? 1 : 0, (uint16_t)width, (uint16_t)height, frame_per_second, error);

    if (ready) {
        printf("VIDEO_STATE: ready=true, width=%u, height=%u, fps=%.2f\n",
               width, height, frame_per_second);
    } else {
        printf("VIDEO_STATE: ready=false, error=%s\n", error ? error : "unknown");
    }
}
