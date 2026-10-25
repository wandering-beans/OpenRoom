#include "bridge.h"
#include <algorithm>
#include <array>
#include <chrono>
#include <cmath>
#include <condition_variable>
#include <cstdio>
#include <iostream>
#include <memory>
#include <mutex>
#include <stdexcept>
#include <vector>
#include <unistd.h>
#include <giomm/init.h>
#include <exiv2/exiv2.hpp>
#include "rtengine/rtengine.h"
#include "rtengine/rtapp.h"
#include "rtengine/improccoordinator.h"
#include "rtengine/imagefloat.h"
#include "rtengine/labimage.h"
#include "rtengine/curves.h"
#include "rtengine/color.h"
#include "rtengine/dcp.h"
#include "rtengine/stdimagesource.h"
#include "rtgui/options.h"
#ifdef _OPENMP
#include <omp.h>
#endif

using namespace rtengine;
using namespace rtengine::procparams;
namespace {
thread_local std::string last_error;

// Keep upstream diagnostic output out of the worker's framed stdout protocol.
class Diagnostics {
    int saved;
public:
    Diagnostics() : saved(dup(STDOUT_FILENO)) {
        if (saved < 0 || dup2(STDERR_FILENO, STDOUT_FILENO) < 0) throw std::runtime_error("Cannot redirect engine diagnostics");
    }
    ~Diagnostics() {
        std::cout.flush();
        fflush(stdout);
        dup2(saved, STDOUT_FILENO);
        close(saved);
    }
};

class LinearSource final : public StdImageSource {
public:
    LinearSource(const float* rgba, int width, int height) {
        std::unique_ptr<Imagefloat> image(new Imagefloat(width, height));
        for (int y = 0; y < height; ++y) {
            for (int x = 0; x < width; ++x) {
                const float* pixel = rgba + (size_t(y) * width + x) * 4;
                image->r(y, x) = pixel[0] * 65535.f;
                image->g(y, x) = pixel[1] * 65535.f;
                image->b(y, x) = pixel[2] * 65535.f;
            }
        }
        img = image.release();
        idata = new FramesData("");
        wb = ColorTemp(1., 1., 1., 1., ColorTemp::DEFAULT_OBSERVER);
    }
    void convertColorSpace(Imagefloat*, const ColorManagementParams&, const ColorTemp&) override {}
};

class Session final : public PreviewImageListener {
public:
    InitialImage* initial = nullptr;
    ImProcCoordinator* processor = nullptr;
    const Imagefloat* linear = nullptr;
    std::unique_ptr<Imagefloat> denoised;
    std::array<float, 2> denoise_key{};
    std::unique_ptr<Imagefloat> detailed;
    std::array<float, 7> detail_key{};
    const Imagefloat* output = nullptr;
    float gain = 1.f;
    float profile_gain = 1.f;
    AdobeToneCurve camera_curve;
    std::mutex mutex;
    std::condition_variable ready;
    std::string error;
    bool completed = false;
    bool first = true;
    bool raw;
    int source_width = 0, source_height = 0;
    int detail_threads = 4;
    ColorTemp camera_wb;
    std::string camera;
    ProcParams previous;

    Session(const char* path, const float* rgba, uint32_t width, uint32_t height, uint32_t bound) : raw(!rgba) {
        App::get().mut_options().prevdemo = bound == 384 ? PD_Fast : PD_Sidecar;
        int code = 0;
        initial = rgba ? static_cast<InitialImage*>(new LinearSource(rgba, width, height)) : InitialImage::load(path, true, &code);
        if (!initial || code) throw std::runtime_error("rtengine cannot decode the photo (" + std::to_string(code) + ")");
        initial->getImageSource()->getFullSize(source_width, source_height);
        if (source_width <= 0 || source_height <= 0 || uint64_t(source_width) * source_height > 64000000) {
            initial->decreaseRef();
            initial = nullptr;
            throw std::runtime_error("Development input exceeds 64 megapixels or is empty");
        }
        detail_threads = !bound || bound >= uint32_t(std::max(source_width, source_height)) ? 8 : 4;
        camera_wb = initial->getImageSource()->getWB();
        camera = initial->getMetaData()->getCamera();
        processor = static_cast<ImProcCoordinator*>(StagedImageProcessor::create(initial));
        // create() transfers the initial reference; its destructor releases it.
        processor->openroomLinearOnly = true;
        processor->setPreviewImageListener(this);
        int skip = bound ? std::max(1, std::max(source_width, source_height) / int(bound)) : 1;
        processor->setPreviewScale(skip);
    }
    ~Session() override {
        if (processor) {
            processor->stopProcessing();
            StagedImageProcessor::destroy(processor);
        }
    }
    void setImage(IImage8*, double, const CropParams&, const CropGuideParams&) override {}
    void delImage(IImage8* image) override { delete image; }
    void imageReady(const CropParams&, const CropGuideParams&) override {
        std::lock_guard<std::mutex> lock(mutex);
        try { linear = processor->openroomFloat(); }
        catch (const std::exception& e) { error = e.what(); }
        completed = true;
        ready.notify_one();
    }

    void render(const OpenRoomAdjustments& a, OpenRoomFrame& out) {
        const bool repeated_render = output != nullptr;
#ifdef _OPENMP
        // Full-size detail can use more cores without increasing preview contention.
        omp_set_num_threads(std::min(detail_threads, omp_get_num_procs()));
#endif
        ProcParams params;
        params.icm.workingProfile = "sRGB";
        params.icm.outputProfile = "No ICM: sRGB output";
        params.icm.inputProfile = raw ? "(cameraICC)" : "(none)";
        params.icm.toneCurve = false;
        params.icm.applyLookTable = false;
        params.icm.applyHueSatMap = true;
        params.icm.applyBaselineExposureOffset = true;
        params.toneCurve.autoexp = false;
        // Exposure is a linear gain after denoising; it needs no display-tone update.
        params.toneCurve.expcomp = 0.;
        // Reconstruct clipped sensor channels before the camera matrix amplifies their imbalance.
        params.toneCurve.hrenabled = raw;
        params.toneCurve.method = "Blend";
        params.toneCurve.clampOOG = false;
        params.wb.method = "Custom";
        const double base_temp = a.raw_temperature > 0 ? a.raw_temperature : camera_wb.getTemp();
        const double temp = base_temp > 0 ? base_temp : 6504.;
        params.wb.temperature = std::round(std::max(1500., std::min(60000., temp * std::pow(2., a.raw_temperature > 0 ? 0. : a.temperature / 200.))));
        params.wb.green = a.raw_temperature > 0 ? std::pow(2., -a.raw_tint / 150.)
            : camera_wb.getGreen() * std::pow(2., -a.tint / 150.);
        params.wb.green = std::max(0.02, std::min(100., params.wb.green));
        params.wb.observer = ColorTemp::DEFAULT_OBSERVER;
        params.wb.enabled = raw || a.temperature != 0 || a.tint != 0;
        if (first && raw) {
            // Fit once to the embedded camera JPEG; slider updates reuse the same curve.
            try {
                std::vector<double> points;
                initial->getImageSource()->getAutoMatchedToneCurve(params.icm, params.raw, params.wb.observer, points);
                if (points.size() > 1) {
                    DiagonalCurve curve(points);
                    camera_curve.Set(curve, Color::sRGBGammaCurve);
                    float previous_value = 0.f;
                    for (int i = 0; i < 65536; ++i) {
                        const float value = camera_curve.lutToneCurve[i];
                        if (!std::isfinite(value)) {
                            camera_curve.Reset();
                            break;
                        }
                        // Spline overshoot and float rounding must not reverse highlight levels.
                        previous_value = std::max(previous_value, std::min(value, 65535.f));
                        camera_curve.lutToneCurve[i] = previous_value;
                    }
                }
            } catch (const std::exception&) {
                // A damaged or missing JPEG must not prevent sensor-data development.
                camera_curve.Reset();
            }
            DCPProfileApplyState state;
            if (auto* profile = initial->getImageSource()->getDCP(params.icm, state)) {
                // The early linear hook skips DCP step 2; fold its exposure-only gain into output.
                float red = 1.f, green = 1.f, blue = 1.f;
                profile->step2ApplyTile(&red, &green, &blue, 1, 1, 1, state);
                profile_gain = red;
            }
        }
        if (first || params != previous) {
            {
                std::lock_guard<std::mutex> lock(mutex);
                completed = false;
                error.clear();
            }
            ProcParams* updated = processor->beginUpdateParams();
            *updated = params;
            processor->endUpdateParams(ProcEvent(first ? EvPhotoLoaded : params.wb != previous.wb ? EvWBTemp : EvExpComp));
            std::unique_lock<std::mutex> lock(mutex);
            if (!ready.wait_for(lock, std::chrono::seconds(90), [this] { return completed; })) throw std::runtime_error("rtengine preview timed out");
            if (!error.empty()) throw std::runtime_error(error);
            previous = params;
            first = false;
            detailed.reset();
            denoised.reset();
        }
        if (!linear) throw std::runtime_error("rtengine returned no floating-point image");
        // RAW stages can be larger than the final preview because their skip is integral.
        const float detail_scale = raw ? float(std::max(linear->getWidth(), linear->getHeight())) / std::max(source_width, source_height) : a.scale;
        const std::array<float, 7> key{{a.sharpen, a.sharpen_radius, a.sharpen_detail, a.sharpen_masking, a.noise_luma, a.noise_color, detail_scale}};
        const bool needs_detail = a.sharpen > 0 || a.noise_luma > 0 || a.noise_color > 0;
        if (needs_detail && (!detailed || key != detail_key)) {
            detailed.reset();
            output = nullptr;
            std::unique_ptr<Imagefloat> result(new Imagefloat(linear->getWidth(), linear->getHeight()));
            ProcParams detail_params;
            detail_params.icm.workingProfile = "sRGB";
            detail_params.sharpening.enabled = a.sharpen > 0;
            detail_params.sharpening.radius = std::max(0.3f, a.sharpen_radius * detail_scale);
            detail_params.sharpening.amount = std::round(a.sharpen * (0.4f + a.sharpen_detail * 0.006f));
            detail_params.sharpening.contrast = a.sharpen_masking;
            detail_params.dirpyrDenoise.enabled = a.noise_luma > 0 || a.noise_color > 0;
            detail_params.dirpyrDenoise.luma = a.noise_luma;
            detail_params.dirpyrDenoise.chroma = a.noise_color;
            detail_params.dirpyrDenoise.Cmethod = "MAN";
            detail_params.dirpyrDenoise.C2method = "MANU";
            detail_params.dirpyrDenoise.autoGain = false;
            ImProcFunctions functions(&detail_params, true);
            functions.setScale(1.);
            const std::array<float, 2> noise_key{{a.noise_luma, a.noise_color}};
            const bool needs_denoise = detail_params.dirpyrDenoise.enabled && result->getWidth() >= 32 && result->getHeight() >= 32;
            if (!needs_denoise || noise_key != denoise_key) denoised.reset();
            if (denoised) denoised->copyData(result.get());
            else linear->copyData(result.get());
            if (needs_denoise && !denoised) {
                NoiseCurve luma, chroma;
                // OpenRoom exposes manual strengths, so no curve-analysis image is needed.
                luma.Reset();
                chroma.Reset();
                float residual = 0, high_residual = 0;
                functions.RGB_denoise(2, result.get(), result.get(), nullptr, nullptr, nullptr, nullptr,
                    false, detail_params.dirpyrDenoise, 0., luma, chroma, residual, high_residual);
                // One-shot exports need no cache; repeated previews keep at most 384 MB.
                if (repeated_render && detail_params.sharpening.enabled && size_t(result->getWidth()) * result->getHeight() <= 32000000) {
                    denoised.reset(new Imagefloat(result->getWidth(), result->getHeight()));
                    result->copyData(denoised.get());
                    denoise_key = noise_key;
                }
            }
            if (detail_params.sharpening.enabled) {
                LabImage lab(result->getWidth(), result->getHeight());
                functions.rgb2lab(*result, lab, "sRGB");
                functions.sharpening(&lab, detail_params.sharpening);
                functions.lab2rgb(lab, *result, "sRGB");
            }
            detailed = std::move(result);
            detail_key = key;
        }
        if (!needs_detail) { detailed.reset(); denoised.reset(); }
        output = needs_detail ? detailed.get() : linear;
        gain = std::pow(2.f, a.exposure) * profile_gain / 65535.f;
        out = {uint32_t(output->getWidth()), uint32_t(output->getHeight()), uint32_t(source_width), uint32_t(source_height), camera_wb.getTemp(), camera_wb.getGreen()};
    }

    void copy_output(float* pixels, size_t count) const {
        if (!output || !pixels || count != size_t(output->getWidth()) * output->getHeight() * 4) {
            throw std::runtime_error("Invalid rtengine output buffer");
        }
        const int width = output->getWidth(), height = output->getHeight();
#ifdef _OPENMP
#pragma omp parallel for schedule(static)
#endif
        for (int y = 0; y < height; ++y) {
            for (int x = 0; x < width; ++x) {
                float* p = pixels + (size_t(y) * width + x) * 4;
                float r = output->r(y, x) * gain;
                float g = output->g(y, x) * gain;
                float b = output->b(y, x) * gain;
                if (camera_curve) {
                    r *= 65535.f;
                    g *= 65535.f;
                    b *= 65535.f;
                    const float low = std::min({r, g, b}), high = std::max({r, g, b});
                    if (low >= 0.f && high <= 65535.f) {
                        camera_curve.Apply(r, g, b);
                    } else {
                        // Extend the film-like mapping without clipping RAW highlight headroom.
                        const auto map = [this](float value) {
                            return value < 0.f || value > 65535.f ? value : camera_curve.lutToneCurve[value];
                        };
                        const float mapped_low = map(low), mapped_high = map(high);
                        const float slope = high > low ? (mapped_high - mapped_low) / (high - low) : 1.f;
                        r = mapped_low + (r - low) * slope;
                        g = mapped_low + (g - low) * slope;
                        b = mapped_low + (b - low) * slope;
                    }
                    r /= 65535.f;
                    g /= 65535.f;
                    b /= 65535.f;
                }
                p[0] = r;
                p[1] = g;
                p[2] = b;
                p[3] = 1.f;
            }
        }
    }
};
}

extern "C" int openroom_rt_init(const char* data, const char* cache) {
    try {
        Diagnostics diagnostics;
        Gio::init();
#ifdef _OPENMP
        omp_set_num_threads(4);
        omp_set_max_active_levels(1);
#endif
        auto& options = App::get().mut_options();
        // Bound denoise scratch memory and process overlapping tiles in parallel.
        options.rgbDenoiseThreadLimit = 4;
        options.rtSettings.verbose = false;
        options.rtSettings.enableLibRaw = true;
        options.rtSettings.iccDirectory = "";
        options.rtSettings.metadata_xmp_sync = Settings::MetadataXmpSync::NONE;
        rtengine::init(&options.rtSettings, data, cache, false);
        return 0;
    } catch (const std::exception& e) { last_error = e.what(); return -1; }
}
extern "C" void* openroom_rt_open(const char* path, const float* rgba, uint32_t width, uint32_t height, uint32_t bound) {
    try {
        Diagnostics diagnostics;
        return new Session(path, rgba, width, height, bound);
    } catch (const std::exception& e) { last_error = e.what(); return nullptr; }
}
extern "C" int openroom_rt_render(void* session, const OpenRoomAdjustments* adjustments, OpenRoomFrame* frame) {
    try {
        Diagnostics diagnostics;
        if (!session || !adjustments || !frame) throw std::runtime_error("Invalid rtengine request");
        static_cast<Session*>(session)->render(*adjustments, *frame);
        return 0;
    } catch (const std::exception& e) { last_error = e.what(); return -1; }
}
extern "C" int openroom_rt_copy(void* session, float* pixels, size_t count) {
    try {
        if (!session) throw std::runtime_error("Invalid rtengine session");
        static_cast<Session*>(session)->copy_output(pixels, count);
        return 0;
    } catch (const std::exception& e) { last_error = e.what(); return -1; }
}
extern "C" void openroom_rt_close(void* session) { delete static_cast<Session*>(session); }
extern "C" const char* openroom_rt_camera(void* session) { return static_cast<Session*>(session)->camera.c_str(); }
extern "C" int openroom_rt_camera_matched(void* session) { return bool(static_cast<Session*>(session)->camera_curve); }
extern "C" const char* openroom_rt_error() { return last_error.c_str(); }

extern "C" double openroom_rt_raster_temperature(const char* source) {
    try {
        Diagnostics diagnostics;
        auto image = Exiv2::ImageFactory::open(source);
        if (!image.get()) return 0.;
        image->readMetadata();
        const auto& exif = image->exifData();
        const auto value = [&exif](const char* key) -> double {
            auto entry = exif.findKey(Exiv2::ExifKey(key));
            return entry != exif.end() && entry->count() == 1 ? entry->toFloat() : 0.;
        };
        // Canon/Fujifilm keep unused manual temperatures in auto-WB files.
        if (value("Exif.CanonSi.WhiteBalance") == 9) {
            for (const char* key : {"Exif.Canon.ColorTemperature", "Exif.CanonPr.ColorTemperature"}) {
                const double kelvin = value(key);
                if (kelvin >= 2000. && kelvin <= 60000.) return kelvin;
            }
        }
        if (value("Exif.Fujifilm.WhiteBalance") == 0xff0) {
            const double kelvin = value("Exif.Fujifilm.ColorTemperature");
            if (kelvin >= 2000. && kelvin <= 60000.) return kelvin;
        }
        for (const char* key : {"Exif.Pentax.ColorTemperature", "Exif.PentaxDng.ColorTemperature",
                               "Exif.OlympusCs.WhiteBalanceTemperature", "Exif.Sony1.ColorTemperature",
                               "Exif.Sony2.ColorTemperature"}) {
            const double kelvin = value(key);
            if (kelvin >= 2000. && kelvin <= 60000.) return kelvin;
        }
    } catch (const std::exception&) {
        // Optional or damaged metadata must not prevent image development.
    }
    return 0.;
}
