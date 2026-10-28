#pragma once
#include <cstddef>
#include <cstdint>

struct OpenRoomAdjustments {
    float exposure, temperature, tint, raw_temperature, raw_tint;
    float sharpen, sharpen_radius, sharpen_detail, sharpen_masking;
    float noise_luma, noise_color, scale;
    uint32_t ca_auto, defringe;
    float ca_red, ca_blue, defringe_radius, defringe_threshold;
};
struct OpenRoomFrame {
    uint32_t width, height, source_width, source_height;
    double temperature, green;
};
extern "C" {
int openroom_rt_init(const char* data, const char* cache);
void* openroom_rt_open(const char* source, const float* rgba, uint32_t width, uint32_t height, uint32_t bound);
int openroom_rt_render(void* session, const OpenRoomAdjustments* adjustments, OpenRoomFrame* result);
int openroom_rt_copy(void* session, float* pixels, size_t count);
void openroom_rt_close(void* session);
const char* openroom_rt_camera(void* session);
int openroom_rt_camera_matched(void* session);
int openroom_rt_auto_ca_supported(void* session);
const char* openroom_rt_error();
double openroom_rt_raster_temperature(const char* source);
}
