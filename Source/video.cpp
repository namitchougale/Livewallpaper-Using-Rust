// Each decoder belongs to a single worker thread. All frames are silent video frames.
#include <windows.h>
#include <mfapi.h>
#include <mferror.h>
#include <mfidl.h>
#include <mfreadwrite.h>
#include <new>
#include <cstdint>
#include <cstring>

template<class T> static void release(T*& object) {
    if (object) { object->Release(); object = nullptr; }
}

struct Decoder {
    HMODULE platform, readwrite;
    decltype(&MFShutdown) shutdown;
    decltype(&MFCreateAttributes) attributes;
    decltype(&MFCreateMediaType) media_type;
    decltype(&MFCreateFile) open_file;
    decltype(&MFCreateSourceReaderFromByteStream) reader_from_stream;
    decltype(&MFCreateSourceReaderFromURL) reader_from_file;
    IMFSourceReader* reader;
    IMFSample* sample;
    IMFMediaBuffer* buffer;
    IMF2DBuffer* surface;
    bool com, started, locked, locked2d;
    UINT32 width, height, max_long, max_short;
    LONG stride;
    double aspect;
    LONGLONG frame_duration;
};

struct FrameView {
    const BYTE* first_row;
    LONG width, height, stride;
    double aspect;
    LONGLONG timestamp, duration;
};

extern "C" void decoder_unlock(Decoder* d) {
    if (d->locked2d) d->surface->Unlock2D();
    if (d->locked) d->buffer->Unlock();
    d->locked2d = d->locked = false;
    release(d->surface); release(d->buffer); release(d->sample);
}
extern "C" void decoder_close(Decoder* d) {
    if (!d) return;
    decoder_unlock(d);
    release(d->reader);
    if (d->started) d->shutdown();
    if (d->readwrite) FreeLibrary(d->readwrite);
    if (d->platform) FreeLibrary(d->platform);
    if (d->com) CoUninitialize();
    d->~Decoder();
    HeapFree(GetProcessHeap(), 0, d);
}

static HRESULT initialize(Decoder* d) {
    HRESULT hr = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    if (FAILED(hr)) return hr;
    d->com = true;
    d->platform = LoadLibraryExW(L"mfplat.dll", nullptr, LOAD_LIBRARY_SEARCH_SYSTEM32);
    d->readwrite = LoadLibraryExW(L"mfreadwrite.dll", nullptr, LOAD_LIBRARY_SEARCH_SYSTEM32);
    if (!d->platform || !d->readwrite) return HRESULT_FROM_WIN32(ERROR_MOD_NOT_FOUND);
    auto startup = reinterpret_cast<decltype(&MFStartup)>(GetProcAddress(d->platform, "MFStartup"));
    d->shutdown = reinterpret_cast<decltype(d->shutdown)>(GetProcAddress(d->platform,"MFShutdown"));
    d->attributes = reinterpret_cast<decltype(d->attributes)>(GetProcAddress(d->platform,"MFCreateAttributes"));
    d->media_type = reinterpret_cast<decltype(d->media_type)>(GetProcAddress(d->platform,"MFCreateMediaType"));
    d->open_file = reinterpret_cast<decltype(d->open_file)>(GetProcAddress(d->platform,"MFCreateFile"));
    d->reader_from_stream = reinterpret_cast<decltype(d->reader_from_stream)>(GetProcAddress(d->readwrite,"MFCreateSourceReaderFromByteStream"));
    d->reader_from_file = reinterpret_cast<decltype(d->reader_from_file)>(GetProcAddress(d->readwrite,"MFCreateSourceReaderFromURL"));
    if (!startup || !d->shutdown || !d->attributes || !d->media_type || !d->reader_from_file)
        return HRESULT_FROM_WIN32(ERROR_PROC_NOT_FOUND);
    hr = startup(MF_VERSION, MFSTARTUP_FULL);
    d->started = SUCCEEDED(hr);
    return hr;
}

static HRESULT update_format(Decoder* d) {
    IMFMediaType* type = nullptr;
    HRESULT hr = d->reader->GetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM,&type);
    if (FAILED(hr)) return hr;
    UINT64 size = 0, par = 0, fps = 0;
    UINT32 stride = 0;
    GUID format = {};
    hr = type->GetGUID(MF_MT_SUBTYPE,&format);
    if (SUCCEEDED(hr) && !IsEqualGUID(format,MFVideoFormat_RGB32)) hr=MF_E_INVALIDMEDIATYPE;
    if (SUCCEEDED(hr)) hr = type->GetUINT64(MF_MT_FRAME_SIZE,&size);
    type->GetUINT64(MF_MT_PIXEL_ASPECT_RATIO,&par);
    type->GetUINT64(MF_MT_FRAME_RATE,&fps);
    type->GetUINT32(MF_MT_DEFAULT_STRIDE,&stride);
    type->Release();
    if (FAILED(hr)) return hr;
    d->width = static_cast<UINT32>(size>>32);
    d->height = static_cast<UINT32>(size);
    if (!d->width || !d->height || d->width>16384 || d->height>16384 ||
        UINT64(d->width)*d->height>16777216) return MF_E_INVALIDMEDIATYPE;
    // Some transforms ignore the requested output size. Avoid a multi-frame 4K RGB queue
    // under the process ceiling; a properly downscaled 4K input still works.
    if (UINT64(d->width)*d->height > 1920ULL*1080) return E_OUTOFMEMORY;
    d->stride = stride ? static_cast<LONG>(stride) : -static_cast<LONG>(4*d->width);
    d->aspect = double(d->width)/d->height;
    if ((par>>32) && UINT32(par)) d->aspect *= double(par>>32)/UINT32(par);
    d->frame_duration = (fps>>32) && UINT32(fps) ? 10000000LL*UINT32(fps)/(fps>>32) : 333333;
    return S_OK;
}

static HRESULT configure(Decoder* d,const wchar_t* path,bool enhanced) {
    IMFAttributes* options = nullptr;
    HRESULT hr = d->attributes(&options, 2);
    if (FAILED(hr)) return hr;
    hr = options->SetUINT32(enhanced ? MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING : MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, TRUE);
    if (enhanced && SUCCEEDED(hr)) hr = options->SetUINT32(MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, TRUE);
    if (SUCCEEDED(hr) && enhanced && d->open_file && d->reader_from_stream) {
        // A filesystem path can contain characters with special meanings in a URL.
        IMFByteStream* stream=nullptr;
        hr=d->open_file(MF_ACCESSMODE_READ,MF_OPENMODE_FAIL_IF_NOT_EXIST,MF_FILEFLAGS_NONE,path,&stream);
        if (SUCCEEDED(hr)) {
            IMFAttributes* metadata=nullptr;
            if (SUCCEEDED(stream->QueryInterface(IID_IMFAttributes,reinterpret_cast<void**>(&metadata)))) {
                metadata->SetString(MF_BYTESTREAM_CONTENT_TYPE,L"video/mp4");
                metadata->Release();
            }
            hr=d->reader_from_stream(stream,options,&d->reader);
            stream->Release();
        }
    } else if (SUCCEEDED(hr)) {
        hr=d->reader_from_file(path,options,&d->reader);
    }
    options->Release();
    if (FAILED(hr)) return hr;
    hr = d->reader->SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS,FALSE);
    if (SUCCEEDED(hr)) hr = d->reader->SetStreamSelection(MF_SOURCE_READER_FIRST_VIDEO_STREAM,TRUE);
    if (FAILED(hr)) return hr;
    IMFMediaType* output = nullptr;
    hr = d->media_type(&output);
    if (FAILED(hr)) return hr;
    hr = output->SetGUID(MF_MT_MAJOR_TYPE,MFMediaType_Video);
    if (SUCCEEDED(hr)) hr = output->SetGUID(MF_MT_SUBTYPE,MFVideoFormat_RGB32);
    // Large clips can be scaled by the Windows video processor before the CPU frame copy.
    if (enhanced && SUCCEEDED(hr)) {
        IMFMediaType* native = nullptr;
        if (SUCCEEDED(d->reader->GetNativeMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM,0,&native))) {
            UINT64 size = 0;
            if (SUCCEEDED(native->GetUINT64(MF_MT_FRAME_SIZE,&size))) {
                UINT32 x = size>>32, y = UINT32(size);
                const UINT32 longest = x>y ? x:y, shortest = x>y ? y:x;
                if (x && y && (longest>d->max_long || shortest>d->max_short)) {
                    const double sx = double(d->max_long)/longest, sy = double(d->max_short)/shortest;
                    const double scale = sx<sy ? sx:sy;
                    const UINT32 w = (UINT32(x*scale)/2)*2, h = (UINT32(y*scale)/2)*2;
                    if (w && h) output->SetUINT64(MF_MT_FRAME_SIZE,(UINT64(w)<<32)|h);
                }
            }
            native->Release();
        }
    }
    if (SUCCEEDED(hr)) hr = d->reader->SetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM,nullptr,output);
    output->Release();
    return FAILED(hr) ? hr : update_format(d);
}

extern "C" HRESULT decoder_open(const wchar_t* path, BOOL enhanced, UINT32 max_long, UINT32 max_short, Decoder** out) {
    *out = nullptr;
    void* memory=HeapAlloc(GetProcessHeap(),0,sizeof(Decoder));
    if (!memory) return E_OUTOFMEMORY;
    auto* d = new(memory) Decoder{};
    d->max_long=max_long; d->max_short=max_short;
    HRESULT hr = initialize(d);
    if (SUCCEEDED(hr)) hr = configure(d,path,enhanced!=FALSE);
    if (FAILED(hr)) {decoder_close(d);return hr;}
    *out=d;return S_OK;
}

// S_OK = timed sample, S_FALSE = end of stream, 2 = reader tick without a sample.
extern "C" HRESULT decoder_read(Decoder* d, FrameView* frame) {
    decoder_unlock(d);
    DWORD flags=0; LONGLONG pts=0;
    HRESULT hr=d->reader->ReadSample(MF_SOURCE_READER_FIRST_VIDEO_STREAM,0,nullptr,&flags,&pts,&d->sample);
    if (FAILED(hr)) return hr;
    if (flags & MF_SOURCE_READERF_ERROR) return E_FAIL;
    if (flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED) { hr=update_format(d); if (FAILED(hr)) return hr; }
    if (!d->sample) return (flags & MF_SOURCE_READERF_ENDOFSTREAM) ? S_FALSE : 2;
    LONGLONG duration=d->frame_duration;
    if (FAILED(d->sample->GetSampleDuration(&duration)) || duration<=0) duration=d->frame_duration;
    *frame={nullptr,LONG(d->width),LONG(d->height),0,d->aspect,pts,duration};
    return S_OK;
}

extern "C" HRESULT decoder_lock(Decoder* d, FrameView* frame) {
    HRESULT hr=d->sample->ConvertToContiguousBuffer(&d->buffer);
    if (FAILED(hr)) return hr;
    BYTE* top=nullptr; LONG pitch=d->stride;
    if (SUCCEEDED(d->buffer->QueryInterface(IID_IMF2DBuffer,reinterpret_cast<void**>(&d->surface)))) {
        d->locked2d=SUCCEEDED(d->surface->Lock2D(&top,&pitch));
    }
    if (!d->locked2d) {
        BYTE* base=nullptr;DWORD length=0;
        hr=d->buffer->Lock(&base,nullptr,&length);
        if (FAILED(hr)) return hr;
        d->locked=true;pitch=d->stride;
        const UINT64 abs_pitch=pitch<0?-INT64(pitch):pitch;
        const UINT64 tail=abs_pitch*(d->height-1);
        if (!base || tail+4ULL*d->width>length) return MF_E_BUFFERTOOSMALL;
        top=base+(pitch<0 ? tail : 0);
    }
    if (!top || (pitch<0?-INT64(pitch):pitch)<4LL*d->width) return MF_E_BUFFERTOOSMALL;
    frame->first_row=top;
    frame->stride=pitch;
    return S_OK;
}
extern "C" HRESULT decoder_rewind(Decoder* d) {
    decoder_unlock(d);
    PROPVARIANT pos={};pos.vt=VT_I8;pos.hVal.QuadPart=0;
    return d->reader->SetCurrentPosition(GUID_NULL,pos);
}
