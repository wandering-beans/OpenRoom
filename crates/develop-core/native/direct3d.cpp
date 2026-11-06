// Synchronous color compute. Never modify the caller's pixels until readback succeeds.
#define NOMINMAX
#include <d3d11.h>
#include <d3dcompiler.h>
#include <dxgi1_6.h>
#include <wrl/client.h>
#include <algorithm>
#include <cstdio>
#include <cstring>
#include <mutex>

using Microsoft::WRL::ComPtr;
namespace {
struct ImageBuffers {
    ComPtr<ID3D11Buffer> image, readback;
    ComPtr<ID3D11UnorderedAccessView> view;
    UINT bytes = 0;
};
struct ColorGPU {
    ComPtr<ID3D11Device> device;
    ComPtr<ID3D11DeviceContext> context;
    ComPtr<IDXGIDevice2> completion;
    HANDLE completed = nullptr;
    ~ColorGPU() { if (completed) CloseHandle(completed); }
    ComPtr<ID3D11ComputeShader> shader;
    ComPtr<ID3D11Buffer> transfer, parameters;
    ComPtr<ID3D11ShaderResourceView> transferView;
    ImageBuffers preview;
    bool attempted = false;
    std::mutex mutex;

    bool table(const float* data, size_t bytes, ComPtr<ID3D11Buffer>& buffer,
               ComPtr<ID3D11ShaderResourceView>& view) {
        D3D11_BUFFER_DESC desc = {};
        desc.ByteWidth = static_cast<UINT>(bytes);
        desc.Usage = D3D11_USAGE_IMMUTABLE;
        desc.BindFlags = D3D11_BIND_SHADER_RESOURCE;
        desc.MiscFlags = D3D11_RESOURCE_MISC_BUFFER_STRUCTURED;
        desc.StructureByteStride = sizeof(float);
        D3D11_SUBRESOURCE_DATA initial = {data, 0, 0};
        return SUCCEEDED(device->CreateBuffer(&desc, &initial, &buffer))
            && SUCCEEDED(device->CreateShaderResourceView(buffer.Get(), nullptr, &view));
    }
    bool initialize(const char* source, size_t length, const float* tables, size_t tableBytes) {
        if (attempted) return shader != nullptr;
        attempted = true;
        ComPtr<IDXGIFactory6> factory;
        if (FAILED(CreateDXGIFactory1(IID_PPV_ARGS(&factory)))) return false;
        // Prefer the discrete GPU on hybrid laptops, but try the integrated GPU if needed.
        for (UINT index = 0; ; ++index) {
            ComPtr<IDXGIAdapter1> adapter;
            if (FAILED(factory->EnumAdapterByGpuPreference(index, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE,
                                                          IID_PPV_ARGS(&adapter)))) break;
            DXGI_ADAPTER_DESC1 description = {};
            if (FAILED(adapter->GetDesc1(&description)) || (description.Flags & DXGI_ADAPTER_FLAG_SOFTWARE)) continue;
            const D3D_FEATURE_LEVEL levels[] = {D3D_FEATURE_LEVEL_11_0};
            if (SUCCEEDED(D3D11CreateDevice(adapter.Get(), D3D_DRIVER_TYPE_UNKNOWN, nullptr, 0,
                          levels, 1, D3D11_SDK_VERSION, &device, nullptr, &context))) {
                std::fprintf(stderr, "OpenRoom Direct3D color processing: %ls\n", description.Description);
                break;
            }
        }
        if (!device || FAILED(device.As(&completion))) return false;
        completed = CreateEventW(nullptr, FALSE, FALSE, nullptr);
        if (!completed) return false;
        ComPtr<ID3DBlob> bytecode, errors;
        if (FAILED(D3DCompile(source, length, "OpenRoom color.hlsl", nullptr, nullptr, "correct_color", "cs_5_0",
                             D3DCOMPILE_IEEE_STRICTNESS | D3DCOMPILE_OPTIMIZATION_LEVEL3, 0, &bytecode, &errors))) {
            if (errors) std::fprintf(stderr, "OpenRoom Direct3D shader: %s\n",
                                    static_cast<const char*>(errors->GetBufferPointer()));
            return false;
        }
        if (!table(tables, tableBytes, transfer, transferView)) return false;
        D3D11_BUFFER_DESC desc = {};
        desc.ByteWidth = 27 * 16;
        desc.Usage = D3D11_USAGE_DEFAULT;
        desc.BindFlags = D3D11_BIND_CONSTANT_BUFFER;
        if (FAILED(device->CreateBuffer(&desc, nullptr, &parameters))) return false;
        return SUCCEEDED(device->CreateComputeShader(bytecode->GetBufferPointer(), bytecode->GetBufferSize(), nullptr, &shader));
    }
    bool allocate(ImageBuffers& buffers, UINT bytes) {
        if (buffers.bytes == bytes) return true;
        buffers = {};
        D3D11_BUFFER_DESC desc = {};
        desc.ByteWidth = bytes;
        desc.Usage = D3D11_USAGE_DEFAULT;
        desc.BindFlags = D3D11_BIND_UNORDERED_ACCESS;
        desc.MiscFlags = D3D11_RESOURCE_MISC_BUFFER_STRUCTURED;
        desc.StructureByteStride = 4 * sizeof(float);
        if (FAILED(device->CreateBuffer(&desc, nullptr, &buffers.image))
            || FAILED(device->CreateUnorderedAccessView(buffers.image.Get(), nullptr, &buffers.view))) return false;
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
        desc.MiscFlags = 0;
        desc.StructureByteStride = 0;
        if (FAILED(device->CreateBuffer(&desc, nullptr, &buffers.readback))) return false;
        buffers.bytes = bytes;
        return true;
    }
};
}
extern "C" bool openroom_d3d_color(
    float* pixels, size_t pixelBytes, const void* parameters, size_t parameterBytes,
    const float* tone, size_t toneBytes, const float* curves, size_t curveBytes,
    const float* transfer, size_t transferBytes, const char* source, size_t sourceBytes) {
    if (!pixels || !parameters || !tone || !curves || !transfer || !source
        || parameterBytes != 27 * 16 || pixelBytes == 0 || pixelBytes % 16
        || pixelBytes > 64000000ULL * 16 || toneBytes % 4 || curveBytes % 4
        || toneBytes < 4 || toneBytes > 4097 * 4 || curveBytes < 4 * 4097 * 4
        || curveBytes > 8 * 4097 * 4 || transferBytes != (4097 + 3 * 65537) * 4
        || sourceBytes == 0) return false;
    UINT dimensions[4];
    std::memcpy(dimensions, parameters, sizeof(dimensions));
    if (dimensions[0] == 0 || dimensions[1] == 0
        || static_cast<unsigned long long>(dimensions[0]) * dimensions[1] != pixelBytes / 16
        || ((dimensions[2] & 1) && toneBytes != 4097 * 4)
        || ((dimensions[2] & 1024) && curveBytes != 8 * 4097 * 4)) return false;
    static ColorGPU gpu;
    std::lock_guard<std::mutex> lock(gpu.mutex);
    if (!gpu.initialize(source, sourceBytes, transfer, transferBytes)) return false;
    // Retain only display-sized buffers; exports must release their large allocations.
    ImageBuffers full;
    ImageBuffers& image = pixelBytes <= 32 * 1024 * 1024 ? gpu.preview : full;
    if (!gpu.allocate(image, static_cast<UINT>(pixelBytes))) return false;
    ComPtr<ID3D11Buffer> toneBuffer, curveBuffer;
    ComPtr<ID3D11ShaderResourceView> toneView, curveView;
    if (!gpu.table(tone, toneBytes, toneBuffer, toneView)
        || !gpu.table(curves, curveBytes, curveBuffer, curveView)) return false;
    gpu.context->UpdateSubresource(image.image.Get(), 0, nullptr, pixels, 0, 0);
    gpu.context->UpdateSubresource(gpu.parameters.Get(), 0, nullptr, parameters, 0, 0);
    ID3D11ShaderResourceView* views[] = {toneView.Get(), curveView.Get(), gpu.transferView.Get()};
    ID3D11Buffer* constants[] = {gpu.parameters.Get()};
    ID3D11UnorderedAccessView* output[] = {image.view.Get()};
    gpu.context->CSSetShader(gpu.shader.Get(), nullptr, 0);
    gpu.context->CSSetConstantBuffers(0, 1, constants);
    gpu.context->CSSetShaderResources(0, 3, views);
    gpu.context->CSSetUnorderedAccessViews(0, 1, output, nullptr);
    const UINT groups = static_cast<UINT>((pixelBytes / 16 + 255) / 256);
    gpu.context->Dispatch(std::min(groups, 65535u), (groups + 65534) / 65535, 1);
    // Unbind transient resources so the context does not retain full-resolution buffers.
    ID3D11UnorderedAccessView* emptyOutput[] = {nullptr};
    ID3D11ShaderResourceView* emptyViews[] = {nullptr, nullptr, nullptr};
    gpu.context->CSSetUnorderedAccessViews(0, 1, emptyOutput, nullptr);
    gpu.context->CSSetShaderResources(0, 3, emptyViews);
    gpu.context->CopyResource(image.readback.Get(), image.image.Get());
    // Driver completion events avoid timer-granularity delays and CPU polling.
    HRESULT result = gpu.completion->EnqueueSetEvent(gpu.completed);
    if (SUCCEEDED(result) && WaitForSingleObject(gpu.completed, 5000) != WAIT_OBJECT_0)
        result = DXGI_ERROR_DEVICE_HUNG;
    if (SUCCEEDED(result)) result = gpu.device->GetDeviceRemovedReason();
    D3D11_MAPPED_SUBRESOURCE mapped = {};
    if (SUCCEEDED(result))
        result = gpu.context->Map(image.readback.Get(), 0, D3D11_MAP_READ,
                                   0, &mapped);
    if (FAILED(result)) {
        std::fprintf(stderr, "OpenRoom Direct3D readback failed: 0x%08lx; using CPU\n", static_cast<unsigned long>(result));
        // A lost or stalled device stays on CPU until the resident worker is restarted.
        gpu.shader.Reset();
        gpu.context->ClearState();
        gpu.preview = {};
        return false;
    }
    std::memcpy(pixels, mapped.pData, pixelBytes);
    gpu.context->Unmap(image.readback.Get(), 0);
    return true;
}
