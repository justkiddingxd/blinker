// Screen recording: ScreenCaptureKit streams (screen, desktop sound,
// microphone), an AVAssetWriter MP4 encoder (VideoToolbox hardware H.264 /
// HEVC + AAC tracks), pixel buffer helpers and the instant replay remux.
//
// Time crosses into Rust in 100 ns units ("hns"), host clock based: the same
// clock ScreenCaptureKit stamps its samples with.

#import <AVFoundation/AVFoundation.h>
#import <CoreMedia/CoreMedia.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <VideoToolbox/VideoToolbox.h>
#include "bk.h"

#define HNS 10000000LL
#define RATE 48000

SCShareableContent *bk_shareable_content(NSError **error);
SCDisplay *bk_find_display(SCShareableContent *c, uint32_t id);

static int64_t to_hns(CMTime t) {
    if (!CMTIME_IS_NUMERIC(t)) return 0;
    return CMTimeConvertScale(t, (int32_t)HNS, kCMTimeRoundingMethod_RoundHalfAwayFromZero).value;
}

int64_t bk_host_time_hns(void) { return to_hns(CMClockGetTime(CMClockGetHostTimeClock())); }

void bk_cf_retain(const void *p) {
    if (p) CFRetain(p);
}

void bk_cf_release(const void *p) {
    if (p) CFRelease(p);
}

// ---------------------------------------------------------------------------
// Pixel buffers (BGRA)

void *bk_pixbuf_new(int w, int h) {
    NSDictionary *attrs = @{(id)kCVPixelBufferIOSurfacePropertiesKey : @{}};
    CVPixelBufferRef pb = NULL;
    if (CVPixelBufferCreate(kCFAllocatorDefault, (size_t)w, (size_t)h, kCVPixelFormatType_32BGRA, (__bridge CFDictionaryRef)attrs, &pb) !=
        kCVReturnSuccess)
        return NULL;
    return pb;
}

/// Locks for CPU access; returns the base address, row stride and size.
uint8_t *bk_pixbuf_lock(void *p, int readonly, size_t *stride, int *w, int *h) {
    CVPixelBufferRef pb = (CVPixelBufferRef)p;
    if (CVPixelBufferLockBaseAddress(pb, readonly ? kCVPixelBufferLock_ReadOnly : 0) != kCVReturnSuccess) return NULL;
    *stride = CVPixelBufferGetBytesPerRow(pb);
    *w = (int)CVPixelBufferGetWidth(pb);
    *h = (int)CVPixelBufferGetHeight(pb);
    return CVPixelBufferGetBaseAddress(pb);
}

void bk_pixbuf_unlock(void *p, int readonly) {
    CVPixelBufferUnlockBaseAddress((CVPixelBufferRef)p, readonly ? kCVPixelBufferLock_ReadOnly : 0);
}

void *bk_pixbuf_copy(void *p) {
    @autoreleasepool {
    CVPixelBufferRef src = (CVPixelBufferRef)p;
    int w = (int)CVPixelBufferGetWidth(src), h = (int)CVPixelBufferGetHeight(src);
    CVPixelBufferRef dst = bk_pixbuf_new(w, h);
    if (!dst) return NULL;
    size_t ss, ds;
    int sw, sh, dw, dh;
    uint8_t *s = bk_pixbuf_lock(src, 1, &ss, &sw, &sh);
    uint8_t *d = bk_pixbuf_lock(dst, 0, &ds, &dw, &dh);
    if (s && d) {
        size_t row = (size_t)w * 4;
        for (int y = 0; y < h; y++) memcpy(d + y * ds, s + y * ss, row);
    }
    if (s) bk_pixbuf_unlock(src, 1);
    if (d) bk_pixbuf_unlock(dst, 0);
    return dst;
    }
}

// ---------------------------------------------------------------------------
// Audio: any PCM layout -> 48 kHz stereo 16-bit interleaved

typedef struct {
    double pos;      // next output position, in input frames of the coming buffer
    float prev[2];   // last frame of the previous buffer
    double rate;
} resampler;

/// `in`: `n` stereo float frames at `r->rate`. Appends to `out` (int16 pairs).
static void resample(resampler *r, const float *in, int n, NSMutableData *out) {
    if (n <= 0) return;
    if (fabs(r->rate - RATE) < 0.5) {
        size_t at = out.length / 2;
        [out increaseLengthBy:(NSUInteger)n * 4];
        int16_t *o = (int16_t *)[out mutableBytes] + at;
        for (int i = 0; i < n * 2; i++) {
            float v = in[i] * 32767.0f;
            o[i] = (int16_t)(v > 32767.0f ? 32767.0f : v < -32768.0f ? -32768.0f : v);
        }
        return;
    }
    double step = r->rate / RATE;
    double p = r->pos;
    while (p <= n - 1) {
        int i = (int)floor(p);
        double f = p - i;
        float a[2], b[2];
        for (int c = 0; c < 2; c++) {
            a[c] = i < 0 ? r->prev[c] : in[i * 2 + c];
            b[c] = (i + 1 < n) ? in[(i + 1) * 2 + c] : a[c];
        }
        int16_t pair[2];
        for (int c = 0; c < 2; c++) {
            float v = (float)(a[c] + (b[c] - a[c]) * f) * 32767.0f;
            pair[c] = (int16_t)(v > 32767.0f ? 32767.0f : v < -32768.0f ? -32768.0f : v);
        }
        [out appendBytes:pair length:4];
        p += step;
    }
    r->pos = p - n;
    r->prev[0] = in[(n - 1) * 2];
    r->prev[1] = in[(n - 1) * 2 + 1];
}

/// Stereo float frames of an audio sample buffer (mono is doubled, extra
/// channels dropped). Returns the input sample rate (0 on failure).
static double audio_frames(CMSampleBufferRef sb, NSMutableData *floats) {
    CMFormatDescriptionRef fd = CMSampleBufferGetFormatDescription(sb);
    const AudioStreamBasicDescription *asbd = fd ? CMAudioFormatDescriptionGetStreamBasicDescription(fd) : NULL;
    if (!asbd || asbd->mFormatID != kAudioFormatLinearPCM) return 0;
    size_t need = 0;
    CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(sb, &need, NULL, 0, NULL, NULL, 0, NULL);
    if (need == 0) return 0;
    AudioBufferList *abl = malloc(need);
    CMBlockBufferRef block = NULL;
    OSStatus st = CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(sb, NULL, abl, need, NULL, NULL,
                                                                          kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment, &block);
    if (st != noErr) {
        free(abl);
        return 0;
    }
    int frames = (int)CMSampleBufferGetNumSamples(sb);
    BOOL isFloat = (asbd->mFormatFlags & kAudioFormatFlagIsFloat) != 0;
    BOOL planar = (asbd->mFormatFlags & kAudioFormatFlagIsNonInterleaved) != 0;
    int ch = (int)asbd->mChannelsPerFrame;
    int bits = (int)asbd->mBitsPerChannel;
    [floats setLength:(NSUInteger)frames * 2 * sizeof(float)];
    float *o = floats.mutableBytes;
    for (int i = 0; i < frames; i++) {
        for (int c = 0; c < 2; c++) {
            int sc = c < ch ? c : 0;
            const void *base;
            size_t index;
            if (planar) {
                UInt32 bi = (UInt32)sc < abl->mNumberBuffers ? (UInt32)sc : 0;
                base = abl->mBuffers[bi].mData;
                index = (size_t)i;
            } else {
                base = abl->mBuffers[0].mData;
                index = (size_t)i * ch + sc;
            }
            float v = 0;
            if (base) {
                if (isFloat && bits == 32) v = ((const float *)base)[index];
                else if (!isFloat && bits == 16) v = ((const int16_t *)base)[index] / 32768.0f;
                else if (!isFloat && bits == 32) v = ((const int32_t *)base)[index] / 2147483648.0f;
            }
            o[i * 2 + c] = v;
        }
    }
    if (block) CFRelease(block);
    free(abl);
    return asbd->mSampleRate;
}

// ---------------------------------------------------------------------------
// Capture streams

typedef void (*bk_video_cb)(void *ctx, void *pixbuf, int64_t t);
typedef void (*bk_audio_cb)(void *ctx, const int16_t *pcm, int frames, int64_t t);

typedef struct {
    uint32_t display;
    uint32_t window;  // non-zero: capture this window wherever it is
    double sx, sy, sw, sh;  // source rect, display-local points
    int width, height;      // output pixels
    int fps;
    int cursor;
    int kind;  // 0 screen, 1 desktop sound, 2 microphone
} bk_stream_cfg;

@interface BKStream : NSObject <SCStreamOutput, SCStreamDelegate>
@property(nonatomic, strong) SCStream *stream;
@property(nonatomic, strong) dispatch_queue_t queue;
@property(nonatomic) bk_video_cb vcb;
@property(nonatomic) bk_audio_cb acb;
@property(nonatomic) void *ctx;
@property(nonatomic) int kind;
@property(atomic) BOOL stopped;
@end

@implementation BKStream {
    resampler _rs;
    NSMutableData *_floats;
    NSMutableData *_pcm;
}

- (instancetype)init {
    if ((self = [super init])) {
        _floats = [NSMutableData new];
        _pcm = [NSMutableData new];
        _rs.pos = 0;
        _rs.rate = RATE;
    }
    return self;
}

- (void)stream:(SCStream *)stream didOutputSampleBuffer:(CMSampleBufferRef)sb ofType:(SCStreamOutputType)type {
    (void)stream;
    if (self.stopped || !CMSampleBufferIsValid(sb)) return;
    if (type == SCStreamOutputTypeScreen) {
        if (self.kind != 0 || !self.vcb) return;
        CFArrayRef att = CMSampleBufferGetSampleAttachmentsArray(sb, false);
        if (!att || CFArrayGetCount(att) == 0) return;
        NSDictionary *a = (__bridge NSDictionary *)CFArrayGetValueAtIndex(att, 0);
        NSNumber *status = a[SCStreamFrameInfoStatus];
        if (!status || status.integerValue != SCFrameStatusComplete) return;
        CVPixelBufferRef pb = CMSampleBufferGetImageBuffer(sb);
        if (!pb) return;
        self.vcb(self.ctx, pb, to_hns(CMSampleBufferGetPresentationTimeStamp(sb)));
        return;
    }
    BOOL wanted = (self.kind == 1 && type == SCStreamOutputTypeAudio);
    if (@available(macOS 15.0, *)) {
        if (self.kind == 2 && type == SCStreamOutputTypeMicrophone) wanted = YES;
    }
    if (!wanted || !self.acb) return;
    double rate = audio_frames(sb, _floats);
    if (rate <= 0) return;
    if (fabs(rate - _rs.rate) > 0.5) {
        _rs.rate = rate;
        _rs.pos = 0;
    }
    [_pcm setLength:0];
    int n = (int)(_floats.length / (2 * sizeof(float)));
    resample(&_rs, _floats.bytes, n, _pcm);
    int frames = (int)(_pcm.length / 4);
    if (frames > 0) self.acb(self.ctx, _pcm.bytes, frames, to_hns(CMSampleBufferGetPresentationTimeStamp(sb)));
}

- (void)stream:(SCStream *)stream didStopWithError:(NSError *)error {
    (void)stream;
    NSLog(@"[blinker] capture stopped: %@", error.localizedDescription);
}
@end

/// The microphone picked in the settings (uniqueID); empty = the default.
static NSString *g_mic;

void bk_set_mic(const char *id) {
    NSString *s = id ? [NSString stringWithUTF8String:id] : @"";
    @synchronized([BKStream class]) {
        g_mic = s;
    }
}

static AVCaptureDevice *chosen_mic(void) {
    NSString *want;
    @synchronized([BKStream class]) {
        want = g_mic;
    }
    AVCaptureDevice *d = want.length ? [AVCaptureDevice deviceWithUniqueID:want] : nil;
    // one that's gone: the default
    return d ?: [AVCaptureDevice defaultDeviceWithMediaType:AVMediaTypeAudio];
}

typedef void (*bk_mic_cb)(void *ctx, const char *id, const char *name);

void bk_mic_list(bk_mic_cb cb, void *ctx) {
    @autoreleasepool {
        AVCaptureDeviceDiscoverySession *s =
            [AVCaptureDeviceDiscoverySession discoverySessionWithDeviceTypes:@[AVCaptureDeviceTypeMicrophone, AVCaptureDeviceTypeExternal]
                                                                   mediaType:AVMediaTypeAudio
                                                                    position:AVCaptureDevicePositionUnspecified];
        for (AVCaptureDevice *d in s.devices) cb(ctx, d.uniqueID.UTF8String, d.localizedName.UTF8String);
    }
}

int bk_mic_available(void) {
    if (@available(macOS 15.0, *)) {
        return [AVCaptureDevice defaultDeviceWithMediaType:AVMediaTypeAudio] != nil ? 1 : 0;
    }
    return 0;
}

void *bk_stream_start(const bk_stream_cfg *c, bk_video_cb vcb, bk_audio_cb acb, void *ctx, char *err, int errlen) {
    @autoreleasepool {
        NSError *e = nil;
        SCShareableContent *content = bk_shareable_content(&e);
        if (!content) {
            BK_ERR(err, errlen, @"%@", e.localizedDescription ?: @"no access to the screen");
            return NULL;
        }
        SCDisplay *display = bk_find_display(content, c->display);
        if (!display) {
            BK_ERR(err, errlen, @"no display");
            return NULL;
        }
        SCContentFilter *filter = nil;
        if (c->window && c->kind == 0) {
            for (SCWindow *w in content.windows) {
                if (w.windowID == c->window) {
                    filter = [[SCContentFilter alloc] initWithDesktopIndependentWindow:w];
                    break;
                }
            }
            if (!filter) {
                BK_ERR(err, errlen, @"the window is gone");
                return NULL;
            }
        } else {
            // our own windows (the recording pill and frame) stay out of the video
            NSMutableArray<SCWindow *> *mine = [NSMutableArray new];
            for (SCWindow *w in content.windows)
                if (w.owningApplication.processID == getpid()) [mine addObject:w];
            filter = [[SCContentFilter alloc] initWithDisplay:display excludingWindows:mine];
        }
        SCStreamConfiguration *cfg = [SCStreamConfiguration new];
        cfg.showsCursor = c->cursor ? YES : NO;
        cfg.pixelFormat = kCVPixelFormatType_32BGRA;
        cfg.colorSpaceName = kCGColorSpaceSRGB;
        if (c->kind == 0) {
            cfg.width = (size_t)c->width;
            cfg.height = (size_t)c->height;
            if (!c->window) cfg.sourceRect = CGRectMake(c->sx, c->sy, c->sw, c->sh);
            cfg.minimumFrameInterval = CMTimeMake(1, c->fps > 0 ? c->fps : 60);
            cfg.queueDepth = 8;
            cfg.captureResolution = SCCaptureResolutionBest;
        } else {
            // sound only: the smallest possible picture, once a second
            cfg.width = 2;
            cfg.height = 2;
            cfg.minimumFrameInterval = CMTimeMake(1, 1);
            cfg.queueDepth = 3;
            cfg.showsCursor = NO;
            cfg.sampleRate = RATE;
            cfg.channelCount = 2;
            if (c->kind == 1) {
                cfg.capturesAudio = YES;
                cfg.excludesCurrentProcessAudio = YES;
            } else if (@available(macOS 15.0, *)) {
                cfg.captureMicrophone = YES;
                AVCaptureDevice *mic = chosen_mic();
                if (mic) cfg.microphoneCaptureDeviceID = mic.uniqueID;
            } else {
                BK_ERR(err, errlen, @"microphone capture needs macOS 15");
                return NULL;
            }
        }
        BKStream *s = [BKStream new];
        s.vcb = vcb;
        s.acb = acb;
        s.ctx = ctx;
        s.kind = c->kind;
        s.queue = dispatch_queue_create(c->kind == 0 ? "blinker.capture.screen" : "blinker.capture.audio", DISPATCH_QUEUE_SERIAL);
        s.stream = [[SCStream alloc] initWithFilter:filter configuration:cfg delegate:s];
        // the screen output is always added: without it ScreenCaptureKit logs every dropped frame
        if (![s.stream addStreamOutput:s type:SCStreamOutputTypeScreen sampleHandlerQueue:s.queue error:&e]) {
            BK_ERR(err, errlen, @"%@", e.localizedDescription);
            return NULL;
        }
        if (c->kind == 1 && ![s.stream addStreamOutput:s type:SCStreamOutputTypeAudio sampleHandlerQueue:s.queue error:&e]) {
            BK_ERR(err, errlen, @"%@", e.localizedDescription);
            return NULL;
        }
        if (c->kind == 2) {
            if (@available(macOS 15.0, *)) {
                if (![s.stream addStreamOutput:s type:SCStreamOutputTypeMicrophone sampleHandlerQueue:s.queue error:&e]) {
                    BK_ERR(err, errlen, @"%@", e.localizedDescription);
                    return NULL;
                }
            }
        }
        dispatch_semaphore_t sem = dispatch_semaphore_create(0);
        __block NSError *serr = nil;
        [s.stream startCaptureWithCompletionHandler:^(NSError *ee) {
            serr = ee;
            dispatch_semaphore_signal(sem);
        }];
        if (dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, 8 * NSEC_PER_SEC)) != 0) {
            BK_ERR(err, errlen, @"capture did not start");
            return NULL;
        }
        if (serr) {
            BK_ERR(err, errlen, @"%@", serr.localizedDescription);
            return NULL;
        }
        return (__bridge_retained void *)s;
    }
}

/// Stops the stream; no callback runs after this returns.
void bk_stream_stop(void *p) {
    @autoreleasepool {
    if (!p) return;
    BKStream *s = (__bridge_transfer BKStream *)p;
    s.stopped = YES;
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    [s.stream stopCaptureWithCompletionHandler:^(NSError *e) {
        (void)e;
        dispatch_semaphore_signal(sem);
    }];
    dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC));
    dispatch_sync(s.queue, ^{
                  });
    s.stream = nil;
    }
}

// ---------------------------------------------------------------------------
// MP4 writer

typedef struct {
    int width, height, fps, bitrate;
    int codec;    // 0 H.264, 1 HEVC
    int quality;  // 1..100 constant quality, 0 = average bitrate
    int keyframe_secs;
} bk_video_spec;

@interface BKWriter : NSObject
@property(nonatomic, strong) AVAssetWriter *writer;
@property(nonatomic, strong) AVAssetWriterInput *video;
@property(nonatomic, strong) AVAssetWriterInputPixelBufferAdaptor *adaptor;
@property(nonatomic, strong) NSMutableArray<AVAssetWriterInput *> *audio;
@property(nonatomic) BOOL realtime;
@property(nonatomic) int width, height;
@property(nonatomic) int64_t end;  // hns
@property(nonatomic, strong) NSLock *lock;
@property(nonatomic) CMAudioFormatDescriptionRef pcm;
@property(nonatomic) BOOL finished;
@property(nonatomic) BOOL audio_done;
@end

@implementation BKWriter
- (void)dealloc {
    if (_pcm) CFRelease(_pcm);
}
@end

static CMAudioFormatDescriptionRef pcm_format(void) {
    AudioStreamBasicDescription asbd = {0};
    asbd.mSampleRate = RATE;
    asbd.mFormatID = kAudioFormatLinearPCM;
    asbd.mFormatFlags = kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked;
    asbd.mChannelsPerFrame = 2;
    asbd.mBitsPerChannel = 16;
    asbd.mBytesPerFrame = 4;
    asbd.mFramesPerPacket = 1;
    asbd.mBytesPerPacket = 4;
    AudioChannelLayout layout = {0};
    layout.mChannelLayoutTag = kAudioChannelLayoutTag_Stereo;
    CMAudioFormatDescriptionRef fd = NULL;
    CMAudioFormatDescriptionCreate(kCFAllocatorDefault, &asbd, sizeof(layout), &layout, 0, NULL, NULL, &fd);
    return fd;
}

static NSDictionary *aac_settings(int track) {
    AudioChannelLayout layout = {0};
    layout.mChannelLayoutTag = kAudioChannelLayoutTag_Stereo;
    return @{
        AVFormatIDKey : @(kAudioFormatMPEG4AAC),
        AVSampleRateKey : @(RATE),
        AVNumberOfChannelsKey : @2,
        AVEncoderBitRateKey : @(track == 0 ? 128000 : 96000),
        AVChannelLayoutKey : [NSData dataWithBytes:&layout length:sizeof(layout)],
    };
}

static BOOL wait_ready(AVAssetWriter *w, AVAssetWriterInput *in, double timeout) {
    double waited = 0;
    while (!in.readyForMoreMediaData) {
        if (w.status != AVAssetWriterStatusWriting || waited >= timeout) return NO;
        usleep(1000);
        waited += 0.001;
    }
    return YES;
}

/// Appends interleaved stereo PCM starting at frame `at` (48 kHz units).
static BOOL append_pcm(AVAssetWriter *w, AVAssetWriterInput *in, CMAudioFormatDescriptionRef fmt, const int16_t *pcm, int frames,
                       int64_t at, double timeout, NSLock *lock) {
    if (frames <= 0) return YES;
    if (!wait_ready(w, in, timeout)) return NO;
    size_t bytes = (size_t)frames * 4;
    CMBlockBufferRef block = NULL;
    if (CMBlockBufferCreateWithMemoryBlock(kCFAllocatorDefault, NULL, bytes, kCFAllocatorDefault, NULL, 0, bytes,
                                           kCMBlockBufferAssureMemoryNowFlag, &block) != noErr)
        return NO;
    CMBlockBufferReplaceDataBytes(pcm, block, 0, bytes);
    CMSampleBufferRef sb = NULL;
    OSStatus st = CMAudioSampleBufferCreateReadyWithPacketDescriptions(kCFAllocatorDefault, block, fmt, frames, CMTimeMake(at, RATE), NULL, &sb);
    CFRelease(block);
    if (st != noErr || !sb) return NO;
    [lock lock];
    BOOL ok = [in appendSampleBuffer:sb];
    [lock unlock];
    CFRelease(sb);
    return ok;
}

int bk_hevc_available(void) {
    @autoreleasepool {
    CFArrayRef list = NULL;
    if (VTCopyVideoEncoderList(NULL, &list) != noErr || !list) return 0;
    int found = 0;
    for (NSDictionary *d in (__bridge NSArray *)list) {
        if ([d[(id)kVTVideoEncoderList_CodecType] unsignedIntValue] == kCMVideoCodecType_HEVC) {
            NSNumber *hw = d[(id)kVTVideoEncoderList_IsHardwareAccelerated];
            if (!hw || hw.boolValue) found = 1;
        }
    }
    CFRelease(list);
    return found;
    }
}

static AVAssetWriter *new_writer(const char *path, BOOL faststart, char *err, int errlen) {
    NSURL *url = [NSURL fileURLWithPath:bk_str(path)];
    [NSFileManager.defaultManager removeItemAtURL:url error:nil];
    NSError *e = nil;
    AVAssetWriter *w = [AVAssetWriter assetWriterWithURL:url fileType:AVFileTypeMPEG4 error:&e];
    if (!w) {
        BK_ERR(err, errlen, @"%@", e.localizedDescription);
        return nil;
    }
    w.shouldOptimizeForNetworkUse = faststart;
    return w;
}

static NSDictionary *video_settings(const bk_video_spec *v) {
    NSMutableDictionary *comp = [@{
        AVVideoExpectedSourceFrameRateKey : @(v->fps),
        AVVideoMaxKeyFrameIntervalKey : @(MAX(1, v->fps * MAX(1, v->keyframe_secs))),
        AVVideoMaxKeyFrameIntervalDurationKey : @(MAX(1, v->keyframe_secs)),
        AVVideoAllowFrameReorderingKey : @NO,
    } mutableCopy];
    if (v->quality > 0) comp[AVVideoQualityKey] = @(MIN(100, v->quality) / 100.0);
    else comp[AVVideoAverageBitRateKey] = @(v->bitrate);
    comp[AVVideoProfileLevelKey] = v->codec == 1 ? (__bridge NSString *)kVTProfileLevel_HEVC_Main_AutoLevel : AVVideoProfileLevelH264HighAutoLevel;
    return @{
        AVVideoCodecKey : v->codec == 1 ? AVVideoCodecTypeHEVC : AVVideoCodecTypeH264,
        AVVideoWidthKey : @(v->width),
        AVVideoHeightKey : @(v->height),
        AVVideoCompressionPropertiesKey : comp,
        AVVideoColorPropertiesKey : @{
            AVVideoColorPrimariesKey : AVVideoColorPrimaries_ITU_R_709_2,
            AVVideoTransferFunctionKey : AVVideoTransferFunction_ITU_R_709_2,
            AVVideoYCbCrMatrixKey : AVVideoYCbCrMatrix_ITU_R_709_2,
        },
    };
}

/// `realtime`: frames come live from a capture (late frames are dropped
/// instead of waited for). Free with `bk_writer_free`.
void *bk_writer_new(const char *path, const bk_video_spec *v, int audio_tracks, int realtime, char *err, int errlen) {
    @autoreleasepool {
        AVAssetWriter *w = new_writer(path, !realtime, err, errlen);
        if (!w) return NULL;
        BKWriter *bw = [BKWriter new];
        bw.writer = w;
        bw.realtime = realtime ? YES : NO;
        bw.lock = [NSLock new];
        bw.audio = [NSMutableArray new];
        bw.pcm = pcm_format();
        if (v) {
            bw.width = v->width;
            bw.height = v->height;
            AVAssetWriterInput *in = nil;
            @try {
                in = [AVAssetWriterInput assetWriterInputWithMediaType:AVMediaTypeVideo outputSettings:video_settings(v)];
            } @catch (NSException *ex) {
                BK_ERR(err, errlen, @"%@", ex.reason);
                return NULL;
            }
            in.expectsMediaDataInRealTime = bw.realtime;
            if (![w canAddInput:in]) {
                BK_ERR(err, errlen, @"the encoder refused these video settings");
                return NULL;
            }
            [w addInput:in];
            bw.video = in;
            bw.adaptor = [AVAssetWriterInputPixelBufferAdaptor
                assetWriterInputPixelBufferAdaptorWithAssetWriterInput:in
                                           sourcePixelBufferAttributes:@{
                                               (id)kCVPixelBufferPixelFormatTypeKey : @(kCVPixelFormatType_32BGRA),
                                               (id)kCVPixelBufferWidthKey : @(v->width),
                                               (id)kCVPixelBufferHeightKey : @(v->height),
                                               (id)kCVPixelBufferIOSurfacePropertiesKey : @{},
                                           }];
        }
        for (int t = 0; t < audio_tracks; t++) {
            AVAssetWriterInput *in = [AVAssetWriterInput assetWriterInputWithMediaType:AVMediaTypeAudio outputSettings:aac_settings(t)];
            in.expectsMediaDataInRealTime = bw.realtime;
            if (![w canAddInput:in]) {
                BK_ERR(err, errlen, @"the encoder refused the audio settings");
                return NULL;
            }
            [w addInput:in];
            [bw.audio addObject:in];
        }
        if (![w startWriting]) {
            BK_ERR(err, errlen, @"%@", w.error.localizedDescription ?: @"cannot start writing");
            return NULL;
        }
        [w startSessionAtSourceTime:kCMTimeZero];
        return (__bridge_retained void *)bw;
    }
}

static void note_end(BKWriter *w, int64_t end) {
    [w.lock lock];
    if (end > w.end) w.end = end;
    [w.lock unlock];
}

int bk_writer_video_pixbuf(void *p, void *pb, int64_t t, int64_t dur) {
    @autoreleasepool {
    BKWriter *w = (__bridge BKWriter *)p;
    if (!w.video || w.finished) return 0;
    if (!wait_ready(w.writer, w.video, w.realtime ? 0.0 : 5.0)) {
        if (w.writer.status == AVAssetWriterStatusFailed) NSLog(@"[blinker] writer failed: %@", w.writer.error);
        return 0;
    }
    [w.lock lock];
    BOOL ok = [w.adaptor appendPixelBuffer:(CVPixelBufferRef)pb withPresentationTime:CMTimeMake(t, (int32_t)HNS)];
    [w.lock unlock];
    if (ok) note_end(w, t + dur);
    return ok ? 1 : 0;
    }
}

/// One frame from memory: top-down BGRA rows of `width * 4` bytes.
int bk_writer_video_bgra(void *p, const uint8_t *data, int64_t t, int64_t dur) {
    @autoreleasepool {
    BKWriter *w = (__bridge BKWriter *)p;
    if (!w.video || w.finished) return 0;
    CVPixelBufferRef pb = NULL;
    if (w.adaptor.pixelBufferPool) CVPixelBufferPoolCreatePixelBuffer(kCFAllocatorDefault, w.adaptor.pixelBufferPool, &pb);
    if (!pb) pb = bk_pixbuf_new(w.width, w.height);
    if (!pb) return 0;
    size_t stride;
    int pw, ph;
    uint8_t *dst = bk_pixbuf_lock(pb, 0, &stride, &pw, &ph);
    if (dst) {
        size_t row = (size_t)MIN(pw, w.width) * 4;
        for (int y = 0; y < MIN(ph, w.height); y++) memcpy(dst + y * stride, data + (size_t)y * w.width * 4, row);
        bk_pixbuf_unlock(pb, 0);
    }
    int ok = bk_writer_video_pixbuf(p, pb, t, dur);
    CVPixelBufferRelease(pb);
    return ok;
    }
}

/// Interleaved stereo 16-bit PCM at 48 kHz starting at `t`.
int bk_writer_audio(void *p, int track, const int16_t *pcm, int frames, int64_t t) {
    @autoreleasepool {
    BKWriter *w = (__bridge BKWriter *)p;
    if (track < 0 || track >= (int)w.audio.count || w.finished || w.audio_done) return 0;
    int64_t at = (int64_t)llround((double)t * RATE / HNS);
    BOOL ok = append_pcm(w.writer, w.audio[(NSUInteger)track], w.pcm, pcm, frames, at, w.realtime ? 0.25 : 5.0, w.lock);
    if (ok) note_end(w, t + (int64_t)frames * HNS / RATE);
    return ok ? 1 : 0;
    }
}

/// No more sound: the writer stops holding the video back for it.
void bk_writer_end_audio(void *p) {
    BKWriter *w = (__bridge BKWriter *)p;
    [w.lock lock];
    if (!w.audio_done && w.writer.status == AVAssetWriterStatusWriting)
        for (AVAssetWriterInput *in in w.audio) [in markAsFinished];
    w.audio_done = YES;
    [w.lock unlock];
}

int bk_writer_finish(void *p, char *err, int errlen) {
    @autoreleasepool {
    BKWriter *w = (__bridge BKWriter *)p;
    if (w.finished) return 1;
    w.finished = YES;
    if (w.writer.status != AVAssetWriterStatusWriting) {
        BK_ERR(err, errlen, @"%@", w.writer.error.localizedDescription ?: @"writer failed");
        return 0;
    }
    [w.lock lock];
    if (w.video) [w.video markAsFinished];
    if (!w.audio_done)
        for (AVAssetWriterInput *in in w.audio) [in markAsFinished];
    if (w.end > 0) [w.writer endSessionAtSourceTime:CMTimeMake(w.end, (int32_t)HNS)];
    [w.lock unlock];
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    [w.writer finishWritingWithCompletionHandler:^{
        dispatch_semaphore_signal(sem);
    }];
    dispatch_semaphore_wait(sem, DISPATCH_TIME_FOREVER);
    if (w.writer.status != AVAssetWriterStatusCompleted) {
        BK_ERR(err, errlen, @"%@", w.writer.error.localizedDescription ?: @"could not finish the file");
        return 0;
    }
    return 1;
    }
}

void bk_writer_free(void *p) {
    @autoreleasepool {
    if (!p) return;
    BKWriter *w = (__bridge_transfer BKWriter *)p;
    if (!w.finished && w.writer.status == AVAssetWriterStatusWriting) [w.writer cancelWriting];
    }
}

// ---------------------------------------------------------------------------
// Instant replay: glue the segments without re-encoding the picture

static AVAssetReader *passthrough_reader(NSString *path, AVAssetReaderTrackOutput **out_output) {
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
    AVURLAsset *asset = [AVURLAsset URLAssetWithURL:[NSURL fileURLWithPath:path] options:nil];
    AVAssetTrack *track = [asset tracksWithMediaType:AVMediaTypeVideo].firstObject;
#pragma clang diagnostic pop
    if (!track) return nil;
    NSError *e = nil;
    AVAssetReader *r = [AVAssetReader assetReaderWithAsset:asset error:&e];
    if (!r) return nil;
    AVAssetReaderTrackOutput *o = [AVAssetReaderTrackOutput assetReaderTrackOutputWithTrack:track outputSettings:nil];
    o.alwaysCopiesSampleData = NO;
    if (![r canAddOutput:o]) return nil;
    [r addOutput:o];
    if (![r startReading]) return nil;
    *out_output = o;
    return r;
}

static BOOL is_sync(CMSampleBufferRef sb) {
    CFArrayRef att = CMSampleBufferGetSampleAttachmentsArray(sb, false);
    if (!att || CFArrayGetCount(att) == 0) return YES;
    CFDictionaryRef d = CFArrayGetValueAtIndex(att, 0);
    CFBooleanRef not_sync = CFDictionaryGetValue(d, kCMSampleAttachmentKey_NotSync);
    return !(not_sync && CFBooleanGetValue(not_sync));
}

/// Finds the stretch to save: `base` = first keyframe at or after `from`
/// (replay clock), `len` = its length up to `to`.
int bk_scan_segments(const char **paths, const int64_t *starts, int n, int64_t from, int64_t to, int64_t *base_out, int64_t *len_out, char *err,
                     int errlen) {
    @autoreleasepool {
        int64_t base = -1, last_end = 0;
        for (int i = 0; i < n; i++) {
            AVAssetReaderTrackOutput *o = nil;
            AVAssetReader *r = passthrough_reader(bk_str(paths[i]), &o);
            if (!r) continue;
            CMSampleBufferRef sb;
            BOOL done = NO;
            while ((sb = [o copyNextSampleBuffer])) {
                if (CMSampleBufferGetNumSamples(sb) == 0) {
                    CFRelease(sb);
                    continue;
                }
                int64_t abs = starts[i] + to_hns(CMSampleBufferGetPresentationTimeStamp(sb));
                int64_t dur = to_hns(CMSampleBufferGetDuration(sb));
                BOOL key = is_sync(sb);
                CFRelease(sb);
                if (abs >= to) {
                    done = YES;
                    break;
                }
                if (base < 0) {
                    if (abs < from || !key) continue;
                    base = abs;
                }
                int64_t e = abs - base + (dur > 0 ? dur : HNS / 60);
                if (e > last_end) last_end = e;
            }
            [r cancelReading];
            if (done) break;
        }
        if (base < 0) {
            BK_ERR(err, errlen, @"no keyframe in the buffer");
            return 0;
        }
        *base_out = base;
        *len_out = MAX(to - base, last_end);
        return 1;
    }
}

/// Writes `dst`: the video samples of `base..end` copied as they are, plus
/// `tracks` AAC tracks encoded from `pcm[i]` (interleaved stereo from `base`).
int bk_remux(const char **paths, const int64_t *starts, int n, int64_t base, int64_t end, const char *dst, const int16_t **pcm,
             const size_t *pcm_frames, int tracks, char *err, int errlen) {
    @autoreleasepool {
        // the format of the first sample tells the writer what it is copying
        CMFormatDescriptionRef hint = NULL;
        for (int i = 0; i < n && !hint; i++) {
            AVAssetReaderTrackOutput *o = nil;
            AVAssetReader *r = passthrough_reader(bk_str(paths[i]), &o);
            if (!r) continue;
            CMSampleBufferRef sb;
            while (!hint && (sb = [o copyNextSampleBuffer])) {
                // marker buffers come without samples or format
                if (CMSampleBufferGetNumSamples(sb) > 0) {
                    hint = CMSampleBufferGetFormatDescription(sb);
                    if (hint) CFRetain(hint);
                }
                CFRelease(sb);
            }
            [r cancelReading];
        }
        if (!hint) {
            BK_ERR(err, errlen, @"the buffer has no video");
            return 0;
        }
        AVAssetWriter *w = new_writer(dst, YES, err, errlen);
        if (!w) {
            CFRelease(hint);
            return 0;
        }
        AVAssetWriterInput *vin = [AVAssetWriterInput assetWriterInputWithMediaType:AVMediaTypeVideo outputSettings:nil sourceFormatHint:hint];
        CFRelease(hint);
        vin.expectsMediaDataInRealTime = NO;
        if (![w canAddInput:vin]) {
            BK_ERR(err, errlen, @"cannot copy this video");
            return 0;
        }
        [w addInput:vin];
        NSMutableArray<AVAssetWriterInput *> *ains = [NSMutableArray new];
        for (int t = 0; t < tracks; t++) {
            AVAssetWriterInput *in = [AVAssetWriterInput assetWriterInputWithMediaType:AVMediaTypeAudio outputSettings:aac_settings(t)];
            if (![w canAddInput:in]) {
                BK_ERR(err, errlen, @"the encoder refused the audio settings");
                return 0;
            }
            [w addInput:in];
            [ains addObject:in];
        }
        if (![w startWriting]) {
            BK_ERR(err, errlen, @"%@", w.error.localizedDescription);
            return 0;
        }
        [w startSessionAtSourceTime:kCMTimeZero];
        CMAudioFormatDescriptionRef fmt = pcm_format();
        NSLock *lock = [NSLock new];
        const int chunk = RATE / 10;  // 100 ms
        __block int64_t k = 0;        // next audio chunk
        __block BOOL audio_done = NO;
        BOOL (^audio_until)(int64_t) = ^BOOL(int64_t until) {
            for (;;) {
                int64_t frame0 = k * chunk;
                int64_t t = frame0 * HNS / RATE;
                if (tracks == 0 || (size_t)frame0 >= pcm_frames[0]) {
                    // all the sound is in: stop the writer waiting for more
                    if (!audio_done)
                        for (AVAssetWriterInput *in in ains) [in markAsFinished];
                    audio_done = YES;
                    return YES;
                }
                if (t > until) return YES;
                for (int tr = 0; tr < tracks; tr++) {
                    if ((size_t)frame0 >= pcm_frames[tr]) continue;
                    int frames = (int)MIN((size_t)chunk, pcm_frames[tr] - (size_t)frame0);
                    if (!append_pcm(w, ains[(NSUInteger)tr], fmt, pcm[tr] + frame0 * 2, frames, frame0, 5.0, lock)) return NO;
                }
                k++;
            }
        };
        BOOL ok = YES, stop = NO;
        int64_t last_end = 0;
        for (int i = 0; i < n && ok && !stop; i++) {
            AVAssetReaderTrackOutput *o = nil;
            AVAssetReader *r = passthrough_reader(bk_str(paths[i]), &o);
            if (!r) continue;
            CMSampleBufferRef sb;
            while (ok && (sb = [o copyNextSampleBuffer])) {
                if (CMSampleBufferGetNumSamples(sb) == 0) {
                    CFRelease(sb);
                    continue;
                }
                int64_t abs = starts[i] + to_hns(CMSampleBufferGetPresentationTimeStamp(sb));
                if (abs >= end) {
                    CFRelease(sb);
                    stop = YES;
                    break;
                }
                if (abs < base) {
                    CFRelease(sb);
                    continue;
                }
                CMSampleTimingInfo timing;
                CMSampleBufferGetSampleTimingInfo(sb, 0, &timing);
                CMTime pts = CMTimeMake(abs - base, (int32_t)HNS);
                timing.presentationTimeStamp = pts;
                if (CMTIME_IS_VALID(timing.decodeTimeStamp)) {
                    int64_t dts = starts[i] + to_hns(timing.decodeTimeStamp) - base;
                    timing.decodeTimeStamp = CMTimeMake(dts < 0 ? 0 : dts, (int32_t)HNS);
                }
                CMSampleBufferRef copy = NULL;
                CMSampleBufferCreateCopyWithNewTiming(kCFAllocatorDefault, sb, 1, &timing, &copy);
                int64_t dur = to_hns(timing.duration);
                CFRelease(sb);
                if (!copy) continue;
                // AVAssetWriter holds the video back until the sound is about a second ahead
                ok = audio_until(abs - base + 2 * HNS);
                if (ok) ok = wait_ready(w, vin, 5.0) && [vin appendSampleBuffer:copy];
                CFRelease(copy);
                int64_t e = abs - base + (dur > 0 ? dur : HNS / 60);
                if (e > last_end) last_end = e;
            }
            [r cancelReading];
        }
        if (ok) ok = audio_until(INT64_MAX);
        CFRelease(fmt);
        if (!ok) {
            BK_ERR(err, errlen, @"%@", w.error.localizedDescription ?: @"could not write the replay");
            [w cancelWriting];
            return 0;
        }
        [vin markAsFinished];
        if (!audio_done)
            for (AVAssetWriterInput *in in ains) [in markAsFinished];
        [w endSessionAtSourceTime:CMTimeMake(MAX(end - base, last_end), (int32_t)HNS)];
        dispatch_semaphore_t sem = dispatch_semaphore_create(0);
        [w finishWritingWithCompletionHandler:^{
            dispatch_semaphore_signal(sem);
        }];
        dispatch_semaphore_wait(sem, DISPATCH_TIME_FOREVER);
        if (w.status != AVAssetWriterStatusCompleted) {
            BK_ERR(err, errlen, @"%@", w.error.localizedDescription ?: @"could not finish the replay");
            return 0;
        }
        return 1;
    }
}
