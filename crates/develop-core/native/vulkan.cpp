// The loader and GPU drivers belong to the host. Absence of either leaves CPU rendering available.
#define VK_NO_PROTOTYPES
#include <vulkan/vulkan_raii.hpp>
#include <algorithm>
#include <array>
#include <cstdio>
#include <cstring>
#include <memory>
#include <mutex>
#include <stdexcept>
#include <vector>

namespace {
constexpr size_t tilePixels = 1024 * 1024;
constexpr size_t pixelStride = 4 * sizeof(float);
constexpr size_t parameterSize = 27 * 16;
constexpr size_t transferSize = (4097 + 3 * 65537) * sizeof(float);
constexpr size_t toneSize = 4097 * sizeof(float);
constexpr size_t curvesSize = 8 * toneSize;
constexpr auto storage = vk::DescriptorType::eStorageBuffer;
constexpr auto uniform = vk::DescriptorType::eUniformBuffer;
constexpr std::array<vk::DescriptorType, 5> types{storage, uniform, storage, storage, storage};

struct Compute {
    vk::raii::Device device;
    vk::raii::Queue queue;
    vk::raii::DescriptorSetLayout layout{nullptr};
    vk::raii::PipelineLayout pipelineLayout{nullptr};
    vk::raii::Pipeline pipeline{nullptr};
    vk::raii::CommandPool pool{nullptr};
    vk::raii::CommandBuffer command{nullptr};
    vk::raii::Fence fence{nullptr};
    // Destroy the buffer before freeing its bound memory.
    vk::raii::DeviceMemory memory{nullptr};
    vk::raii::Buffer buffer{nullptr};
    vk::raii::DeviceMemory stagingMemory{nullptr};
    vk::raii::Buffer staging{nullptr};
    bool transferUploaded = false;
    vk::raii::DescriptorPool descriptors{nullptr};
    vk::raii::DescriptorSet set{nullptr};
    unsigned char* mapped = nullptr;
    std::array<size_t, 5> offset{};

    Compute(const vk::raii::PhysicalDevice& physical, uint32_t family,
            const std::vector<uint32_t>& shader, const float* transfer)
        : device(nullptr), queue(nullptr) {
        const float priority = 1.f;
        vk::DeviceQueueCreateInfo createQueue({}, family, 1, &priority);
        device = physical.createDevice(vk::DeviceCreateInfo({}, createQueue));
        queue = device.getQueue(family, 0);
        const auto limits = physical.getProperties().limits;
        if (limits.maxComputeWorkGroupInvocations < 256 || limits.maxComputeWorkGroupSize[0] < 256)
            throw std::runtime_error("unsupported workgroup size");

        std::array<vk::DescriptorSetLayoutBinding, 5> bindings;
        for (unsigned i = 0; i < bindings.size(); ++i)
            bindings[i] = vk::DescriptorSetLayoutBinding(i, types[i], 1, vk::ShaderStageFlagBits::eCompute);
        layout = device.createDescriptorSetLayout(vk::DescriptorSetLayoutCreateInfo({}, bindings));
        pipelineLayout = device.createPipelineLayout(vk::PipelineLayoutCreateInfo({}, *layout));
        auto module = device.createShaderModule(vk::ShaderModuleCreateInfo({}, shader.size() * 4, shader.data()));
        vk::PipelineShaderStageCreateInfo stage({}, vk::ShaderStageFlagBits::eCompute, *module, "correct_color");
        pipeline = vk::raii::Pipeline(device, nullptr, vk::ComputePipelineCreateInfo({}, stage, *pipelineLayout));
        pool = device.createCommandPool({vk::CommandPoolCreateFlagBits::eResetCommandBuffer, family});
        command = std::move(device.allocateCommandBuffers({*pool, vk::CommandBufferLevel::ePrimary, 1}).front());
        fence = device.createFence({});

        const std::array<size_t, 5> lengths{tilePixels * pixelStride, parameterSize, toneSize, curvesSize, transferSize};
        const size_t alignment = std::max(limits.minUniformBufferOffsetAlignment, limits.minStorageBufferOffsetAlignment);
        size_t size = 0;
        for (unsigned i = 0; i < lengths.size(); ++i) {
            offset[i] = (size + alignment - 1) / alignment * alignment;
            size = offset[i] + lengths[i];
        }
        const auto copyUsage = vk::BufferUsageFlagBits::eTransferSrc | vk::BufferUsageFlagBits::eTransferDst;
        buffer = device.createBuffer({{}, size, copyUsage | vk::BufferUsageFlagBits::eStorageBuffer | vk::BufferUsageFlagBits::eUniformBuffer});
        staging = device.createBuffer({{}, size, copyUsage});
        const auto properties = physical.getMemoryProperties();
        auto allocate = [&](vk::raii::Buffer& target, vk::MemoryPropertyFlags required, vk::MemoryPropertyFlags preferred) {
            const auto requirements = target.getMemoryRequirements();
            uint32_t type = VK_MAX_MEMORY_TYPES;
            for (uint32_t i = 0; i < properties.memoryTypeCount; ++i) {
                auto flags = properties.memoryTypes[i].propertyFlags;
                if (!(requirements.memoryTypeBits & (1u << i)) || (flags & required) != required) continue;
                type = i;
                if ((flags & preferred) == preferred) break;
            }
            if (type == VK_MAX_MEMORY_TYPES) throw std::runtime_error("no compatible Vulkan buffer memory");
            auto allocation = device.allocateMemory({requirements.size, type});
            target.bindMemory(*allocation, 0);
            return allocation;
        };
        memory = allocate(buffer, {}, vk::MemoryPropertyFlagBits::eDeviceLocal);
        // Reading host-visible VRAM directly can take hundreds of milliseconds on a discrete GPU.
        stagingMemory = allocate(staging, vk::MemoryPropertyFlagBits::eHostVisible | vk::MemoryPropertyFlagBits::eHostCoherent,
                                 vk::MemoryPropertyFlagBits::eHostCached);
        mapped = static_cast<unsigned char*>(stagingMemory.mapMemory(0, size));
        std::memcpy(mapped + offset[4], transfer, transferSize);
        std::array<vk::DescriptorPoolSize, 2> sizes{{{storage, 4}, {uniform, 1}}};
        descriptors = device.createDescriptorPool({vk::DescriptorPoolCreateFlagBits::eFreeDescriptorSet, 1, sizes});
        set = std::move(device.allocateDescriptorSets({*descriptors, *layout}).front());
        for (unsigned i = 0; i < lengths.size(); ++i) {
            vk::DescriptorBufferInfo info(*buffer, offset[i], lengths[i]);
            device.updateDescriptorSets(vk::WriteDescriptorSet(*set, i, 0, 1, types[i], nullptr, &info), {});
        }
    }

    void render(float* pixels, size_t count, const void* params, const float* tone, size_t toneBytes,
                const float* curves, size_t curvesBytes) {
        std::memcpy(mapped + offset[1], params, parameterSize);
        std::memcpy(mapped + offset[2], tone, toneBytes);
        std::memcpy(mapped + offset[3], curves, curvesBytes);
        // A failed dispatch must leave the CPU fallback's input completely untouched.
        std::unique_ptr<float[]> output;
        if (count > tilePixels) output.reset(new float[count * 4]);
        for (uint32_t base = 0; base < count; base += tilePixels) {
            const size_t n = std::min(tilePixels, count - base);
            std::memcpy(mapped + offset[1] + 12, &base, sizeof(base));
            std::memcpy(mapped, pixels + base * 4, n * pixelStride);
            command.reset();
            command.begin({vk::CommandBufferUsageFlagBits::eOneTimeSubmit});
            const vk::BufferCopy pixelsRegion(0, 0, n * pixelStride);
            size_t end = offset[1] + parameterSize;
            if (!base) end = transferUploaded ? offset[3] + curvesBytes : offset[4] + transferSize;
            const std::array<vk::BufferCopy, 2> upload{pixelsRegion, vk::BufferCopy(offset[1], offset[1], end - offset[1])};
            command.copyBuffer(*staging, *buffer, upload);
            vk::MemoryBarrier uploaded(vk::AccessFlagBits::eTransferWrite,
                vk::AccessFlagBits::eUniformRead | vk::AccessFlagBits::eShaderRead | vk::AccessFlagBits::eShaderWrite);
            command.pipelineBarrier(vk::PipelineStageFlagBits::eTransfer, vk::PipelineStageFlagBits::eComputeShader,
                                    {}, uploaded, {}, {});
            command.bindPipeline(vk::PipelineBindPoint::eCompute, *pipeline);
            command.bindDescriptorSets(vk::PipelineBindPoint::eCompute, *pipelineLayout, 0, *set, {});
            command.dispatch(static_cast<uint32_t>((n + 255) / 256), 1, 1);
            vk::MemoryBarrier computed(vk::AccessFlagBits::eShaderWrite, vk::AccessFlagBits::eTransferRead);
            command.pipelineBarrier(vk::PipelineStageFlagBits::eComputeShader, vk::PipelineStageFlagBits::eTransfer,
                                    {}, computed, {}, {});
            command.copyBuffer(*buffer, *staging, pixelsRegion);
            vk::MemoryBarrier readback(vk::AccessFlagBits::eTransferWrite, vk::AccessFlagBits::eHostRead);
            command.pipelineBarrier(vk::PipelineStageFlagBits::eTransfer, vk::PipelineStageFlagBits::eHost,
                                    {}, readback, {}, {});
            command.end();
            device.resetFences(*fence);
            queue.submit(vk::SubmitInfo({}, {}, *command), *fence);
            if (device.waitForFences(*fence, true, UINT64_MAX) != vk::Result::eSuccess)
                throw std::runtime_error("Vulkan command did not complete");
            transferUploaded = true;
            std::memcpy(output ? output.get() + base * 4 : pixels, mapped, n * pixelStride);
        }
        if (output) std::memcpy(pixels, output.get(), count * pixelStride);
    }
};

struct Vulkan {
    vk::raii::Context context;
    vk::raii::Instance instance;
    std::unique_ptr<Compute> compute;

    Vulkan(const unsigned char* code, size_t bytes, const float* transfer, bool testCpu)
        : instance(context, vk::InstanceCreateInfo{}) {
        // include_bytes! need not be aligned to a SPIR-V word.
        std::vector<uint32_t> shader(bytes / 4);
        std::memcpy(shader.data(), code, bytes);
        auto devices = instance.enumeratePhysicalDevices();
        for (auto type : {vk::PhysicalDeviceType::eDiscreteGpu, vk::PhysicalDeviceType::eIntegratedGpu,
                          vk::PhysicalDeviceType::eVirtualGpu, vk::PhysicalDeviceType::eCpu}) {
            if (type == vk::PhysicalDeviceType::eCpu && !testCpu) continue;
            for (auto& gpu : devices) {
                const auto properties = gpu.getProperties();
                if (properties.deviceType != type) continue;
                const auto families = gpu.getQueueFamilyProperties();
                for (uint32_t i = 0; i < families.size(); ++i) {
                    if (!(families[i].queueFlags & vk::QueueFlagBits::eCompute)) continue;
                    try {
                        compute = std::make_unique<Compute>(gpu, i, shader, transfer);
                        std::fprintf(stderr, "OpenRoom Vulkan color processing: %s\n", properties.deviceName.data());
                        return;
                    } catch (const std::exception& error) {
                        std::fprintf(stderr, "OpenRoom Vulkan: %s: %s\n", properties.deviceName.data(), error.what());
                        break;
                    }
                }
            }
        }
    }
};
}

extern "C" bool openroom_gpu_color(float* pixels, size_t bytes, const uint32_t* params, size_t paramsBytes,
                                   const float* tone, size_t toneBytes, const float* curves, size_t curvesBytes,
                                   const float* transfer, size_t transferBytes, const unsigned char* shader,
                                   size_t shaderBytes, bool testCpu) noexcept {
    if (!pixels || !params || !tone || !curves || !transfer || !shader || !shaderBytes || shaderBytes % 4
        || paramsBytes != parameterSize || !params[0] || !params[1] || bytes % pixelStride
        || bytes / pixelStride != uint64_t(params[0]) * params[1] || bytes / pixelStride > 64000000
        || transferBytes != transferSize || toneBytes != ((params[2] & 1) ? toneSize : sizeof(float))
        || curvesBytes != ((params[2] & 1024) ? curvesSize : curvesSize / 2)) return false;

    static std::mutex mutex;
    static std::unique_ptr<Vulkan> gpu;
    static bool attempted = false;
    std::lock_guard<std::mutex> lock(mutex);
    try {
        if (!attempted) {
            attempted = true;
            gpu = std::make_unique<Vulkan>(shader, shaderBytes, transfer, testCpu);
        }
        if (!gpu || !gpu->compute) return false;
        gpu->compute->render(pixels, bytes / pixelStride, params, tone, toneBytes, curves, curvesBytes);
        return true;
    } catch (const std::bad_alloc&) {
        // A large export can run out of staging memory while later previews still fit.
        return false;
    } catch (const std::exception& error) {
        std::fprintf(stderr, "OpenRoom Vulkan unavailable: %s; using CPU\n", error.what());
        gpu.reset();
        return false;
    }
}
