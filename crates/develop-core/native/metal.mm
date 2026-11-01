#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#include <mutex>
#include <cstring>

namespace {
struct ColorGPU {
    id<MTLDevice> device;
    id<MTLCommandQueue> queue;
    id<MTLComputePipelineState> pipeline;
    id<MTLBuffer> transfer;
    id<MTLBuffer> preview;
    bool attempted = false;
    std::mutex mutex;

    bool initialize(const char* source, size_t length, const float* tables, size_t tableBytes) {
        if (attempted) return pipeline != nil;
        attempted = true;
        device = MTLCreateSystemDefaultDevice();
        if (!device) return false;
        queue = [device newCommandQueue];
        MTLCompileOptions* options = [MTLCompileOptions new];
        // The compatibility property also selects precise math on macOS 14.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
        options.fastMathEnabled = NO;
#pragma clang diagnostic pop
        NSError* error = nil;
        NSString* text = [[NSString alloc] initWithBytes:source length:length encoding:NSUTF8StringEncoding];
        id<MTLLibrary> library = [device newLibraryWithSource:text options:options error:&error];
        id<MTLFunction> function = [library newFunctionWithName:@"correct_color"];
        if (function) pipeline = [device newComputePipelineStateWithFunction:function error:&error];
        transfer = [device newBufferWithBytes:tables length:tableBytes options:MTLResourceStorageModeShared];
        if (!queue || !transfer || !pipeline) {
            pipeline = nil;
            fprintf(stderr, "OpenRoom Metal unavailable: %s\n", error.localizedDescription.UTF8String ?: "resource allocation failed");
            return false;
        }
        fprintf(stderr, "OpenRoom Metal color processing: %s\n", device.name.UTF8String);
        return true;
    }
};
}

// The input is updated only after successful completion, so a failure can safely use the CPU.
extern "C" bool openroom_metal_color(
    float* pixels, size_t pixelBytes, const void* parameters, size_t parameterBytes,
    const float* tone, size_t toneBytes, const float* curves, size_t curveBytes,
    const float* transfer, size_t transferBytes, const char* source, size_t sourceBytes) {
    @autoreleasepool {
        static ColorGPU gpu;
        std::lock_guard<std::mutex> lock(gpu.mutex);
        if (!gpu.initialize(source, sourceBytes, transfer, transferBytes)
            || pixelBytes == 0 || pixelBytes > gpu.device.maxBufferLength) return false;
        // Keep display-sized buffers; a full-resolution frame must not leave a large GPU cache behind.
        constexpr size_t retainedLimit = 32 * 1024 * 1024;
        id<MTLBuffer> image;
        if (pixelBytes <= retainedLimit) {
            if (!gpu.preview || gpu.preview.length < pixelBytes)
                gpu.preview = [gpu.device newBufferWithLength:pixelBytes options:MTLResourceStorageModeShared];
            image = gpu.preview;
        } else {
            image = [gpu.device newBufferWithLength:pixelBytes options:MTLResourceStorageModeShared];
        }
        id<MTLBuffer> toneBuffer = [gpu.device newBufferWithBytes:tone length:toneBytes options:MTLResourceStorageModeShared];
        id<MTLBuffer> curveBuffer = [gpu.device newBufferWithBytes:curves length:curveBytes options:MTLResourceStorageModeShared];
        if (!image || !toneBuffer || !curveBuffer) return false;
        id<MTLCommandBuffer> command = [gpu.queue commandBuffer];
        id<MTLComputeCommandEncoder> encoder = [command computeCommandEncoder];
        if (!command || !encoder) return false;
        memcpy(image.contents, pixels, pixelBytes);
        command.label = @"OpenRoom color correction";
        [encoder setComputePipelineState:gpu.pipeline];
        [encoder setBuffer:image offset:0 atIndex:0];
        [encoder setBytes:parameters length:parameterBytes atIndex:1];
        [encoder setBuffer:toneBuffer offset:0 atIndex:2];
        [encoder setBuffer:curveBuffer offset:0 atIndex:3];
        [encoder setBuffer:gpu.transfer offset:0 atIndex:4];
        NSUInteger threads = MIN(256, gpu.pipeline.maxTotalThreadsPerThreadgroup);
        [encoder dispatchThreads:MTLSizeMake(pixelBytes / (4 * sizeof(float)), 1, 1)
           threadsPerThreadgroup:MTLSizeMake(threads, 1, 1)];
        [encoder endEncoding];
        [command commit];
        [command waitUntilCompleted];
        if (command.status != MTLCommandBufferStatusCompleted) return false;
        memcpy(pixels, image.contents, pixelBytes);
        return true;
    }
}
