#include <stdio.h>
#include <stdbool.h>
#include <stdlib.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/ioctl.h>
#include <linux/videodev2.h>
#include <errno.h>
#include <sys/klog.h>

#define MAX_EDID_SIZE 256
#define V4L_NODE "/dev/video0"

/**
 * @brief Read the EDID from the display
 *
 * @param edid Buffer to store the EDID data
 * @param max_size Maximum size of the buffer (should be 128 or 256)
 * @return int Number of bytes read on success, -1 on failure
 */
int get_edid(uint8_t *edid, size_t max_size);

/**
 * @brief Set the EDID of the display
 *
 * @param edid The EDID to set, it can be modified
 * @param size The size of the EDID (should be 128 or 256)
 * @return int 0 on success, -1 on failure
 */
int set_edid(uint8_t *edid, size_t size);

/**
 * @brief Get the status of the videocontroller, aka v4l2-ctl --log-status.
 * User should free the returned string
 *
 * @return const char* The status of the videocontroller
 */
const char* videoc_log_status();

void free_videoc_status(char *status);

int get_edid(uint8_t *edid, size_t max_size)
{
    if (edid == NULL) {
        errno = EINVAL;
        return -1;
    }

    if (max_size != 128 && max_size != 256) {
        errno = EINVAL;
        return -1;
    }

    int fd;
    struct v4l2_edid v4l2_edid;

    fd = open(V4L_NODE, O_RDWR);
    if (fd < 0) {
        perror("Failed to open device");
        return -1;
    }

    memset(&v4l2_edid, 0, sizeof(v4l2_edid));
    v4l2_edid.pad = 0;
    v4l2_edid.start_block = 0;
    v4l2_edid.blocks = max_size / 128;
    v4l2_edid.edid = edid;

    if (ioctl(fd, VIDIOC_G_EDID, &v4l2_edid) < 0) {
        perror("Failed to get EDID");
        close(fd);
        return -1;
    }

    close(fd);
    return v4l2_edid.blocks * 128;
}

static void fix_edid_checksum(uint8_t *edid, size_t size)
{
    for (size_t block = 0; block < size / 128; block++) {
        uint8_t sum = 0;
        for (int i = 0; i < 127; i++) {
            sum += edid[block * 128 + i];
        }
        edid[block * 128 + 127] = (uint8_t)(256 - sum);
    }
}

int set_edid(uint8_t *edid, size_t size)
{
    if (edid == NULL) {
        errno = EINVAL;
        return -1;
    }

    if (size != 128 && size != 256) {
        errno = EINVAL;
        return -1;
    }

    int fd;
    struct v4l2_edid v4l2_edid;

    fd = open(V4L_NODE, O_RDWR);
    if (fd < 0) {
        perror("Failed to open device");
        return -1;
    }

    fix_edid_checksum(edid, size);

    memset(&v4l2_edid, 0, sizeof(v4l2_edid));
    v4l2_edid.pad = 0;
    v4l2_edid.start_block = 0;
    v4l2_edid.blocks = size / 128;
    v4l2_edid.edid = edid;

    if (ioctl(fd, VIDIOC_S_EDID, &v4l2_edid) < 0) {
        perror("Failed to set EDID");
        close(fd);
        return -1;
    }

    close(fd);
    return 0;
}

const char *videoc_log_status()
{
    int fd;
    char *buffer = NULL;
    size_t buffer_size = 0;
    ssize_t bytes_read;

    fd = open(V4L_NODE, O_RDWR);
    if (fd < 0) {
        perror("Failed to open device");
        return NULL;
    }

    if (ioctl(fd, VIDIOC_LOG_STATUS) == -1) {
        perror("VIDIOC_LOG_STATUS failed");
        close(fd);
        return NULL;
    }

    close(fd);

    char buf[40960];
    int len = -1;

    len = klogctl(3, buf, sizeof(buf) - 1);

    if (len >= 0) {
        bool found_status = false;
        char *p = buf;
        char *q;

        buf[len] = 0;
        while ((q = strstr(p, "START STATUS"))) {
            found_status = true;
            p = q + 1;
        }
        if (found_status) {
            while (p > buf && *p != '<')
                p--;
            q = p;
            while ((q = strstr(q, "<6>"))) {
                memcpy(q, "   ", 3);
            }
        }
        buffer = strdup(p);
        if (buffer == NULL) {
            perror("Failed to allocate memory for status");
            return NULL;
        }
        return buffer;
    } else {
        printf("Failed to read kernel log\n");
        return NULL;
    }

}

void free_videoc_status(char *status)
{
    if (status != NULL) {
        free(status);
    }
}
