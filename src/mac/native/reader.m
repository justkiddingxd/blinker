// Reading media files (AVFoundation): what is inside, decoded frames scaled
// to a size, and audio tracks as 48 kHz stereo 16-bit PCM.
//
// "Streams" are the file's tracks in file order, like Media Foundation's
// stream indices in the Windows build.

#import <Accelerate/Accelerate.h>
#import <AVFoundation/AVFoundation.h>
#include "bk.h"

#define HNS 10000000LL

#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"

typedef struct {
    int width, height;
    double fps;
    int64_t duration;
    int video_stream;  // -1 = none
    int audio_streams[16];
    int audio_count;
} bk_media_info;

static int64_t to_hns(CMTime t) {
    if (!CMTIME_IS_NUMERIC(t)) return 0;
    return CMTimeConvertScale(t, (int32_t)HNS, kCMTimeRoundingMethod_RoundHalfAwayFromZero).value;
}

static AVURLAsset *open_asset(const char *path) {
    return [AVURLAsset URLAssetWithURL:[NSURL fileURLWithPath:bk_str(path)] options:@{AVURLAssetPreferPreciseDurationAndTimingKey : @YES}];
}

int bk_probe(const char *path, bk_media_info *info, char *err, int errlen) {
    @autoreleasepool {
        AVURLAsset *a = open_asset(path);
        NSArray<AVAssetTrack *> *tracks = a.tracks;
        if (tracks.count == 0) {
            BK_ERR(err, errlen, @"no media in the file");
            return 0;
        }
        memset(info, 0, sizeof(*info));
        info->video_stream = -1;
        for (NSUInteger i = 0; i < tracks.count; i++) {
            AVAssetTrack *t = tracks[i];
            if ([t.mediaType isEqualToString:AVMediaTypeVideo] && info->video_stream < 0) {
                info->video_stream = (int)i;
                CGSize s = t.naturalSize;
                info->width = (int)llround(s.width);
                info->height = (int)llround(s.height);
                info->fps = t.nominalFrameRate > 0 ? t.nominalFrameRate : 30.0;
            } else if ([t.mediaType isEqualToString:AVMediaTypeAudio] && info->audio_count < 16) {
                info->audio_streams[info->audio_count++] = (int)i;
            }
        }
        info->duration = to_hns(a.duration);
        return 1;
    }
}

static AVAssetTrack *track_at(AVURLAsset *a, int index) {
    NSArray<AVAssetTrack *> *tracks = a.tracks;
    return index >= 0 && (NSUInteger)index < tracks.count ? tracks[(NSUInteger)index] : nil;
}

static AVAssetReader *reader_for(AVURLAsset *a, AVAssetTrack *t, NSDictionary *settings, int64_t from, AVAssetReaderTrackOutput **out,
                                 char *err, int errlen) {
    NSError *e = nil;
    AVAssetReader *r = [AVAssetReader assetReaderWithAsset:a error:&e];
    if (!r) {
        BK_ERR(err, errlen, @"%@", e.localizedDescription);
        return nil;
    }
    if (from > 0) r.timeRange = CMTimeRangeMake(CMTimeMake(from, (int32_t)HNS), kCMTimePositiveInfinity);
    AVAssetReaderTrackOutput *o = [AVAssetReaderTrackOutput assetReaderTrackOutputWithTrack:t outputSettings:settings];
    o.alwaysCopiesSampleData = NO;
    if (![r canAddOutput:o]) {
        BK_ERR(err, errlen, @"cannot decode this track");
        return nil;
    }
    [r addOutput:o];
    if (![r startReading]) {
        BK_ERR(err, errlen, @"%@", r.error.localizedDescription ?: @"cannot read the file");
        return nil;
    }
    *out = o;
    return r;
}

// ---------------------------------------------------------------------------
// Video

@interface BKVideoReader : NSObject
@property(nonatomic, strong) AVAssetReader *reader;
@property(nonatomic, strong) AVAssetReaderTrackOutput *output;
@property(nonatomic) int width, height;
@end
@implementation BKVideoReader
@end

/// Decodes the first video track from `from` (hns), frames scaled to `w × h`.
void *bk_vreader_open(const char *path, int w, int h, int64_t from, char *err, int errlen) {
    @autoreleasepool {
        AVURLAsset *a = open_asset(path);
        AVAssetTrack *t = [a tracksWithMediaType:AVMediaTypeVideo].firstObject;
        if (!t) {
            BK_ERR(err, errlen, @"no video in the file");
            return NULL;
        }
        AVAssetReaderTrackOutput *o = nil;
        AVAssetReader *r = reader_for(a, t, @{(id)kCVPixelBufferPixelFormatTypeKey : @(kCVPixelFormatType_32BGRA)}, from, &o, err, errlen);
        if (!r) return NULL;
        BKVideoReader *vr = [BKVideoReader new];
        vr.reader = r;
        vr.output = o;
        vr.width = w;
        vr.height = h;
        return (__bridge_retained void *)vr;
    }
}

/// Next frame as RGBA (`w * h * 4` bytes into `out`): 1, or 0 at the end, -1 on errors.
int bk_vreader_next(void *p, uint8_t *out, int64_t *ts) {
    @autoreleasepool {
        BKVideoReader *vr = (__bridge BKVideoReader *)p;
        for (;;) {
            CMSampleBufferRef sb = [vr.output copyNextSampleBuffer];
            if (!sb) return vr.reader.status == AVAssetReaderStatusFailed ? -1 : 0;
            CVPixelBufferRef pb = CMSampleBufferGetImageBuffer(sb);
            if (!pb) {
                CFRelease(sb);
                continue;
            }
            *ts = to_hns(CMSampleBufferGetPresentationTimeStamp(sb));
            CVPixelBufferLockBaseAddress(pb, kCVPixelBufferLock_ReadOnly);
            vImage_Buffer src = {
                .data = CVPixelBufferGetBaseAddress(pb),
                .height = CVPixelBufferGetHeight(pb),
                .width = CVPixelBufferGetWidth(pb),
                .rowBytes = CVPixelBufferGetBytesPerRow(pb),
            };
            vImage_Buffer dst = {.data = out, .height = (vImagePixelCount)vr.height, .width = (vImagePixelCount)vr.width, .rowBytes = (size_t)vr.width * 4};
            vImage_Error ve;
            if (src.width == dst.width && src.height == dst.height) {
                for (vImagePixelCount y = 0; y < src.height; y++)
                    memcpy((uint8_t *)dst.data + y * dst.rowBytes, (uint8_t *)src.data + y * src.rowBytes, dst.rowBytes);
                ve = kvImageNoError;
            } else {
                ve = vImageScale_ARGB8888(&src, &dst, NULL, kvImageHighQualityResampling);
            }
            CVPixelBufferUnlockBaseAddress(pb, kCVPixelBufferLock_ReadOnly);
            CFRelease(sb);
            if (ve != kvImageNoError) return -1;
            // BGRA -> RGBA, opaque
            const uint8_t map[4] = {2, 1, 0, 3};
            vImagePermuteChannels_ARGB8888(&dst, &dst, map, kvImageNoFlags);
            size_t n = (size_t)vr.width * vr.height;
            for (size_t i = 0; i < n; i++) out[i * 4 + 3] = 255;
            return 1;
        }
    }
}

void bk_vreader_free(void *p) {
    if (!p) return;
    BKVideoReader *vr = (__bridge_transfer BKVideoReader *)p;
    if (vr.reader.status == AVAssetReaderStatusReading) [vr.reader cancelReading];
}

// ---------------------------------------------------------------------------
// Audio

@interface BKAudioReader : NSObject
@property(nonatomic, strong) AVAssetReader *reader;
@property(nonatomic, strong) AVAssetReaderTrackOutput *output;
@property(nonatomic, strong) NSMutableData *pcm;
@end
@implementation BKAudioReader
@end

void *bk_areader_open(const char *path, int stream, int64_t from, char *err, int errlen) {
    @autoreleasepool {
        AVURLAsset *a = open_asset(path);
        AVAssetTrack *t = track_at(a, stream);
        if (!t || ![t.mediaType isEqualToString:AVMediaTypeAudio]) {
            BK_ERR(err, errlen, @"no such audio track");
            return NULL;
        }
        NSDictionary *settings = @{
            AVFormatIDKey : @(kAudioFormatLinearPCM),
            AVSampleRateKey : @48000,
            AVNumberOfChannelsKey : @2,
            AVLinearPCMBitDepthKey : @16,
            AVLinearPCMIsFloatKey : @NO,
            AVLinearPCMIsBigEndianKey : @NO,
            AVLinearPCMIsNonInterleaved : @NO,
        };
        AVAssetReaderTrackOutput *o = nil;
        AVAssetReader *r = reader_for(a, t, settings, from, &o, err, errlen);
        if (!r) return NULL;
        BKAudioReader *ar = [BKAudioReader new];
        ar.reader = r;
        ar.output = o;
        ar.pcm = [NSMutableData new];
        return (__bridge_retained void *)ar;
    }
}

/// Next chunk: `*pcm` (interleaved stereo, valid until the next call),
/// `*frames`, `*ts`. 1, or 0 at the end, -1 on errors.
int bk_areader_next(void *p, const int16_t **pcm, int *frames, int64_t *ts) {
    @autoreleasepool {
        BKAudioReader *ar = (__bridge BKAudioReader *)p;
        for (;;) {
            CMSampleBufferRef sb = [ar.output copyNextSampleBuffer];
            if (!sb) return ar.reader.status == AVAssetReaderStatusFailed ? -1 : 0;
            CMBlockBufferRef bb = CMSampleBufferGetDataBuffer(sb);
            size_t len = bb ? CMBlockBufferGetDataLength(bb) : 0;
            if (len < 4) {
                CFRelease(sb);
                continue;
            }
            [ar.pcm setLength:len];
            CMBlockBufferCopyDataBytes(bb, 0, len, ar.pcm.mutableBytes);
            *ts = to_hns(CMSampleBufferGetPresentationTimeStamp(sb));
            CFRelease(sb);
            *pcm = ar.pcm.bytes;
            *frames = (int)(len / 4);
            return 1;
        }
    }
}

void bk_areader_free(void *p) {
    if (!p) return;
    BKAudioReader *ar = (__bridge_transfer BKAudioReader *)p;
    if (ar.reader.status == AVAssetReaderStatusReading) [ar.reader cancelReading];
}

#pragma clang diagnostic pop
