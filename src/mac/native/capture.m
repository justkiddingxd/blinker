// Screenshots (ScreenCaptureKit) and the list of windows on screen.

#import <AppKit/AppKit.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#include <libproc.h>
#include "bk.h"

static void fill_win(NSDictionary *d, bk_win *w) {
    CGRect r = CGRectZero;
    CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)d[(id)kCGWindowBounds], &r);
    w->x = r.origin.x;
    w->y = r.origin.y;
    w->w = r.size.width;
    w->h = r.size.height;
    w->id = [d[(id)kCGWindowNumber] unsignedIntValue];
    w->pid = [d[(id)kCGWindowOwnerPID] intValue];
    w->layer = [d[(id)kCGWindowLayer] intValue];
    NSNumber *a = d[(id)kCGWindowAlpha];
    w->alpha = a ? a.floatValue : 1.0f;
}

/// Windows on screen, front to back.
int bk_window_list(bk_win *out, int max) {
    @autoreleasepool {
        CFArrayRef list = CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, kCGNullWindowID);
        if (!list) return 0;
        int n = 0;
        for (NSDictionary *d in (__bridge NSArray *)list) {
            if (n >= max) break;
            fill_win(d, &out[n++]);
        }
        CFRelease(list);
        return n;
    }
}

int bk_window_info(uint32_t id, bk_win *out) {
    @autoreleasepool {
        CFArrayRef list = CGWindowListCopyWindowInfo(kCGWindowListOptionIncludingWindow, id);
        if (!list) return 0;
        int ok = 0;
        NSArray *a = (__bridge NSArray *)list;
        if (a.count > 0) {
            fill_win(a[0], out);
            ok = 1;
        }
        CFRelease(list);
        return ok;
    }
}

/// Front window of the active app (not ours).
int bk_frontmost_window(bk_win *out) {
    @autoreleasepool {
        pid_t pid = NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier;
        bk_win list[256];
        int n = bk_window_list(list, 256);
        for (int i = 0; i < n; i++) {
            if (list[i].pid == pid && list[i].layer == 0 && list[i].w > 4 && list[i].h > 4 && pid != getpid()) {
                *out = list[i];
                return 1;
            }
        }
        return 0;
    }
}

/// The process runs blinker's own executable (the tray, settings, editors...).
int bk_pid_is_ours(int pid) {
    static char mine[PROC_PIDPATHINFO_MAXSIZE];
    if (!mine[0] && proc_pidpath(getpid(), mine, sizeof(mine)) <= 0) return pid == getpid();
    char other[PROC_PIDPATHINFO_MAXSIZE];
    if (proc_pidpath(pid, other, sizeof(other)) <= 0) return 0;
    return strcmp(mine, other) == 0;
}

/// Screen recording permission: checked, or asked for (system prompt).
int bk_screen_access(int request) {
    if (CGPreflightScreenCaptureAccess()) return 1;
    return request ? (CGRequestScreenCaptureAccess() ? 1 : 0) : 0;
}

SCShareableContent *bk_shareable_content(NSError **error) {
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    __block SCShareableContent *result = nil;
    __block NSError *err = nil;
    [SCShareableContent getShareableContentExcludingDesktopWindows:NO
                                               onScreenWindowsOnly:YES
                                                 completionHandler:^(SCShareableContent *c, NSError *e) {
                                                     result = c;
                                                     err = e;
                                                     dispatch_semaphore_signal(sem);
                                                 }];
    if (dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, 6 * NSEC_PER_SEC)) != 0) {
        if (error) *error = [NSError errorWithDomain:@"blinker" code:-1 userInfo:@{NSLocalizedDescriptionKey : @"timeout"}];
        return nil;
    }
    if (error) *error = err;
    return result;
}

SCDisplay *bk_find_display(SCShareableContent *c, uint32_t id) {
    for (SCDisplay *d in c.displays)
        if (d.displayID == id) return d;
    return c.displays.firstObject;
}

/// Grabs `sw × sh` points at (sx, sy) of `display` (display-local, top-left)
/// as `pw × ph` RGBA pixels (sRGB). Free the result with `bk_free`.
uint8_t *bk_capture_display(uint32_t display, double sx, double sy, double sw, double sh, int pw, int ph, int cursor, char *err,
                            int errlen) {
    @autoreleasepool {
        if (pw <= 0 || ph <= 0) return NULL;
        NSError *e = nil;
        SCShareableContent *content = bk_shareable_content(&e);
        if (!content) {
            BK_ERR(err, errlen, @"%@", e.localizedDescription ?: @"no access");
            return NULL;
        }
        SCDisplay *d = bk_find_display(content, display);
        if (!d) {
            BK_ERR(err, errlen, @"no display");
            return NULL;
        }
        SCContentFilter *filter = [[SCContentFilter alloc] initWithDisplay:d excludingWindows:@[]];
        SCStreamConfiguration *cfg = [SCStreamConfiguration new];
        cfg.sourceRect = CGRectMake(sx, sy, sw, sh);
        cfg.width = (size_t)pw;
        cfg.height = (size_t)ph;
        cfg.showsCursor = cursor ? YES : NO;
        cfg.pixelFormat = kCVPixelFormatType_32BGRA;
        cfg.colorSpaceName = kCGColorSpaceSRGB;
        cfg.captureResolution = SCCaptureResolutionBest;
        dispatch_semaphore_t sem = dispatch_semaphore_create(0);
        __block CGImageRef image = NULL;
        __block NSError *cerr = nil;
        [SCScreenshotManager captureImageWithFilter:filter
                                      configuration:cfg
                                  completionHandler:^(CGImageRef img, NSError *ee) {
                                      if (img) image = CGImageRetain(img);
                                      cerr = ee;
                                      dispatch_semaphore_signal(sem);
                                  }];
        if (dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, 6 * NSEC_PER_SEC)) != 0 || !image) {
            BK_ERR(err, errlen, @"%@", cerr.localizedDescription ?: @"capture failed");
            if (image) CGImageRelease(image);
            return NULL;
        }
        uint8_t *buf = calloc((size_t)pw * ph, 4);
        CGColorSpaceRef cs = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        CGContextRef ctx = CGBitmapContextCreate(buf, (size_t)pw, (size_t)ph, 8, (size_t)pw * 4, cs,
                                                 kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big);
        CGColorSpaceRelease(cs);
        if (!ctx) {
            CGImageRelease(image);
            free(buf);
            BK_ERR(err, errlen, @"bitmap");
            return NULL;
        }
        CGContextSetBlendMode(ctx, kCGBlendModeCopy);
        CGContextDrawImage(ctx, CGRectMake(0, 0, pw, ph), image);
        CGContextRelease(ctx);
        CGImageRelease(image);
        return buf;
    }
}
