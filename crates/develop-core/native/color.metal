#include <metal_stdlib>
using namespace metal;

// Matches Parameters in gpu.rs. All vectors occupy sixteen bytes.
struct Parameters {
    uint4 dimensions_flags;
    float4 gains;
    float4 saturation;
    float4 vignette;
    float4 grain;
    float4 grading_control;
    float4 white_balance[3];
    float4 to_wide[3];
    float4 from_wide[3];
    float4 grading[4];
    float4 mixer[8];
};
float lookup(device const float* table, float position, uint last) {
    uint i = min(uint(position), last - 1);
    return table[i] + (table[i + 1] - table[i]) * (position - float(i));
}
float linear(float v, device const float* table) {
    if (v <= 0.04045f) return v / 12.92f;
    if (v <= 1.f) return lookup(table, v * 4096.f, 4096);
    return pow((v + 0.055f) / 1.055f, 2.4f);
}
float srgb(float v, device const float* table) {
    if (v <= 0.0031308f) return v * 12.92f;
    if (v <= 1.f) return lookup(table + 4097, v * 65536.f, 65536);
    return 1.055f * pow(v, 1.f / 2.4f) - 0.055f;
}
float3 linear3(float3 v, device const float* table) {
    return float3(linear(v.r, table), linear(v.g, table), linear(v.b, table));
}
float3 srgb3(float3 v, device const float* table) {
    return float3(srgb(v.r, table), srgb(v.g, table), srgb(v.b, table));
}
float luminance(float3 v) { return 0.2126f * v.r + 0.7152f * v.g + 0.0722f * v.b; }
float positive_mod(float x, float m) { float v = fmod(x, m); return v < 0.f ? v + m : v; }
float distance_hue(float a, float b) { return abs(positive_mod(a - b + 180.f, 360.f) - 180.f); }
float3 hsl(float3 rgb) {
    float high = max(rgb.r, max(rgb.g, rgb.b)), low = min(rgb.r, min(rgb.g, rgb.b));
    float delta = high - low, light = (high + low) * 0.5f;
    if (delta < 1e-8f) return float3(0.f, 0.f, light);
    float hue = high == rgb.r ? positive_mod((rgb.g - rgb.b) / delta, 6.f)
        : high == rgb.g ? (rgb.b - rgb.r) / delta + 2.f : (rgb.r - rgb.g) / delta + 4.f;
    return float3(hue * 60.f, delta / max(1.f - abs(2.f * light - 1.f), 1e-8f), light);
}
float3 from_hsl(float hue, float saturation, float light) {
    hue = positive_mod(hue, 360.f) / 60.f;
    float c = (1.f - abs(2.f * light - 1.f)) * saturation;
    float second = c * (1.f - abs(positive_mod(hue, 2.f) - 1.f));
    float3 rgb;
    switch (uint(hue)) {
        case 0: rgb = float3(c, second, 0.f); break;
        case 1: rgb = float3(second, c, 0.f); break;
        case 2: rgb = float3(0.f, c, second); break;
        case 3: rgb = float3(0.f, second, c); break;
        case 4: rgb = float3(second, 0.f, c); break;
        default: rgb = float3(c, 0.f, second); break;
    }
    return rgb + light - c * 0.5f;
}
float3 multiply(constant float4* rows, float3 v) {
    return float3(rows[0].x*v.r + rows[0].y*v.g + rows[0].z*v.b,
                  rows[1].x*v.r + rows[1].y*v.g + rows[1].z*v.b,
                  rows[2].x*v.r + rows[2].y*v.g + rows[2].z*v.b);
}
float curve(float v, uint channel, uint flags, device const float* tables) {
    if ((flags & (16u << channel)) == 0) return v;
    float pos = clamp(v, 0.f, 1.f) * 4096.f;
    uint i = min(uint(pos), 4095u); float fraction = pos - float(i);
    device const float* table = tables + channel * 4097;
    return table[i] * (1.f - fraction) + table[i+1] * fraction + (v - clamp(v, 0.f, 1.f));
}
float smooth(float v) { return v * v * (3.f - 2.f * v); }
float hash_noise(int x, int y) {
    uint v = uint(x) * 374761393u ^ uint(y) * 668265263u;
    v = (v ^ (v >> 13)) * 1274126177u;
    return float(v ^ (v >> 16)) / float(0xffffffffu) * 2.f - 1.f;
}
float grain_noise(float x, float y) {
    int ix = int(floor(x)), iy = int(floor(y));
    float fx = smooth(fract(x)), fy = smooth(fract(y));
    float top = hash_noise(ix, iy) * (1.f-fx) + hash_noise(ix+1, iy) * fx;
    float bottom = hash_noise(ix, iy+1) * (1.f-fx) + hash_noise(ix+1, iy+1) * fx;
    return top * (1.f-fy) + bottom * fy;
}
kernel void correct_color(device float4* pixels [[buffer(0)]],
    constant Parameters& p [[buffer(1)]], device const float* tone [[buffer(2)]],
    device const float* curves [[buffer(3)]], device const float* transfer [[buffer(4)]],
    uint index [[thread_position_in_grid]]) {
    uint width = p.dimensions_flags.x, height = p.dimensions_flags.y, flags = p.dimensions_flags.z;
    if (index >= width * height) return;
    float3 rgb = pixels[index].rgb;
    if (flags & 2u) rgb = multiply(p.white_balance, rgb);
    if (flags & 1024u) {
        // The CPU JPEG calibration packs three WB tables and one tone table after the curves.
        device const float* calibration = curves + 4 * 4097;
        float3 wide = multiply(p.to_wide, rgb);
        float3 balanced;
        for (uint channel = 0; channel < 3; channel++) {
            float value = max(wide[channel], 0.f);
            float position = sqrt(min(value, 1.f)) * 4096.f;
            balanced[channel] = lookup(calibration + channel * 4097, position, 4096)
                + (value > 1.f ? srgb(value, transfer) - 1.f : 0.f);
        }
        float low = min(balanced.r, min(balanced.g, balanced.b));
        float high = max(balanced.r, max(balanced.g, balanced.b));
        device const float* jpegTone = calibration + 3 * 4097;
        float mappedLow = lookup(jpegTone, clamp(low, 0.f, 1.f) * 4096.f, 4096) + max(low - 1.f, 0.f);
        float mappedHigh = lookup(jpegTone, clamp(high, 0.f, 1.f) * 4096.f, 4096) + max(high - 1.f, 0.f);
        float3 toned = high - low > 1e-7f
            ? mappedLow + (balanced - low) / (high - low) * (mappedHigh - mappedLow)
            : float3(mappedLow);
        for (uint channel = 0; channel < 3; channel++) {
            float value = toned[channel];
            toned[channel] = value >= 0.f && value <= 1.f
                ? lookup(transfer, value * 4096.f, 4096) : linear(value, transfer);
        }
        rgb = multiply(p.from_wide, toned);
    }
    rgb *= p.gains.rgb;
    if (flags & 1u) {
        float light = luminance(rgb), pos = sqrt(clamp(light / 16.f, 0.f, 1.f)) * 4096.f;
        uint i = min(uint(pos), 4095u); float fraction = pos - float(i);
        float mapped;
        if (light > 16.f) {
            float last = (4095.f/4096.f)*(4095.f/4096.f)*16.f;
            mapped = tone[4096] + (light-16.f) * ((tone[4096]-tone[4095]) / (16.f-last));
        } else mapped = tone[i] * (1.f-fraction) + tone[i+1] * fraction;
        rgb = light > 1e-8f ? srgb3(rgb * (mapped/light), transfer) : float3(srgb(mapped, transfer));
    } else rgb = srgb3(rgb, transfer);
    if (flags & 240u) {
        float3 wide = srgb3(multiply(p.to_wide, linear3(rgb, transfer)), transfer);
        if (flags & 16u) {
            float low = min(wide.r,min(wide.g,wide.b)), high = max(wide.r,max(wide.g,wide.b));
            float mappedLow = curve(low,0,flags,curves), mappedHigh = curve(high,0,flags,curves);
            wide = high-low > 1e-7f ? mappedLow + (wide-low)/(high-low)*(mappedHigh-mappedLow) : float3(mappedLow);
        }
        wide = float3(curve(wide.r,1,flags,curves),curve(wide.g,2,flags,curves),curve(wide.b,3,flags,curves));
        rgb = srgb3(multiply(p.from_wide, linear3(wide,transfer)),transfer);
    }
    if (p.saturation.x != 1.f || p.saturation.y != 0.f || p.saturation.z != 0.f) {
        float3 linearRgb = linear3(rgb, transfer);
        float neutral = luminance(linearRgb);
        float spread = max(rgb.r,max(rgb.g,rgb.b)) - min(rgb.r,min(rgb.g,rgb.b));
        float protect = 1.f - clamp(1.f-distance_hue(hsl(rgb).x,30.f)/30.f,0.f,1.f)*0.5f;
        float saturation = max(0.f,1.f+p.saturation.y/100.f+p.saturation.z/100.f*(1.f-clamp(spread,0.f,1.f))*protect)*p.saturation.x;
        rgb = srgb3(neutral + (linearRgb-neutral)*saturation,transfer);
    }
    if (flags & 256u) {
        float peak = max(1.f,max(rgb.r,max(rgb.g,rgb.b)));
        float3 h = hsl(clamp(rgb/peak,0.f,1.f)), adjustment = float3(0.f);
        float total = 0.f;
        for (uint channel=0;channel<8;channel++) {
            float weight = max(0.f,1.f-distance_hue(h.x,p.mixer[channel].w)/45.f);
            total += weight; adjustment += p.mixer[channel].xyz * weight;
        }
        if (h.y > 1e-8f && total > 0.f && any(adjustment != float3(0.f))) {
            h.x += adjustment.x/total*0.45f; h.y *= 1.f+adjustment.y/total/100.f;
            float delta = adjustment.z/total/100.f;
            h.z += delta*(delta>=0.f ? 1.f-h.z : h.z);
            rgb = from_hsl(h.x,clamp(h.y,0.f,1.f),clamp(h.z,0.f,1.f))*peak;
        }
    }
    if (flags & 4u) rgb = float3(srgb(luminance(linear3(rgb,transfer)),transfer));
    if (flags & 512u) {
        float light = clamp(luminance(rgb)+p.grading_control.x/200.f,0.f,1.f);
        float exponent = 4.f-p.grading_control.y/100.f*3.f;
        float3 weights = pow(float3(1.f-light,1.f-abs(2.f*light-1.f),light),float3(exponent));
        float total = max(weights.x+weights.y+weights.z,1e-6f);
        for (uint zone=0;zone<4;zone++) {
            float weight = zone==3 ? 1.f : weights[zone]/total;
            rgb += weight*p.grading[zone].rgb;
        }
    }
    float x = (float(index%width)+0.5f)/float(width), y=(float(index/width)+0.5f)/float(height);
    if (p.vignette.x != 0.f) {
        float radius = sqrt((x*2.f-1.f)*(x*2.f-1.f)+(y*2.f-1.f)*(y*2.f-1.f))/sqrt(2.f);
        float start = p.vignette.y/100.f*0.8f, feather=max(p.vignette.z/100.f*(1.f-start),0.01f);
        float mask = smooth(clamp((radius-start)/feather,0.f,1.f));
        rgb = srgb3(linear3(rgb,transfer)*pow(2.f,p.vignette.x/100.f*2.f*mask),transfer);
    }
    if (p.grain.x > 0.f) {
        float size = 1.f+p.grain.y*0.2f;
        float noise = grain_noise(x*4096.f/size,y*4096.f/size)*(1.f-p.grain.z/200.f)
            + grain_noise(x*4096.f,y*4096.f)*p.grain.z/200.f;
        float strength = p.grain.x/100.f*0.14f*(0.3f+(1.f-abs(luminance(rgb)*2.f-1.f))*0.7f);
        rgb += noise*strength;
    }
    pixels[index] = float4(rgb,pixels[index].a);
}
