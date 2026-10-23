// Screenshots (ScreenCaptureKit) and the list of windows on screen.

#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <Vision/Vision.h>
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

typedef void (*bk_ocr_cb)(void *ctx, const char *text, double x, double y, double w, double h);

/// The text in an RGBA picture (Vision, accurate, Russian and English with the
/// language guessed): `cb` per line with its box as fractions of the picture,
/// origin top left. Returns the number of lines, -1 when Vision failed.
int bk_ocr(const uint8_t *rgba, int w, int h, void *ctx, bk_ocr_cb cb) {
    @autoreleasepool {
        CGColorSpaceRef cs = CGColorSpaceCreateDeviceRGB();
        CGContextRef bc = CGBitmapContextCreate((void *)rgba, w, h, 8, (size_t)w * 4, cs, kCGImageAlphaPremultipliedLast);
        CGImageRef img = bc ? CGBitmapContextCreateImage(bc) : NULL;
        int n = -1;
        if (img) {
            VNRecognizeTextRequest *req = [VNRecognizeTextRequest new];
            req.recognitionLevel = VNRequestTextRecognitionLevelAccurate;
            req.usesLanguageCorrection = YES;
            req.automaticallyDetectsLanguage = YES;
            req.recognitionLanguages = @[ @"ru-RU", @"en-US" ];
            VNImageRequestHandler *handler = [[VNImageRequestHandler alloc] initWithCGImage:img options:@{}];
            NSError *err = nil;
            if ([handler performRequests:@[ req ] error:&err]) {
                n = 0;
                for (VNRecognizedTextObservation *o in req.results) {
                    VNRecognizedText *t = [o topCandidates:1].firstObject;
                    if (!t) continue;
                    CGRect b = o.boundingBox;  // fractions, origin bottom left
                    cb(ctx, t.string.UTF8String, b.origin.x, 1.0 - b.origin.y - b.size.height, b.size.width, b.size.height);
                    n++;
                }
            }
            CGImageRelease(img);
        }
        if (bc) CGContextRelease(bc);
        CGColorSpaceRelease(cs);
        return n;
    }
}

/// The Accessibility permission (for parts of windows); `prompt`: ask for
/// it (System Settings opens on the list).
int bk_ax_trusted(int prompt) {
    if (!prompt) return AXIsProcessTrusted() ? 1 : 0;
    NSDictionary *o = @{(__bridge id)kAXTrustedCheckOptionPrompt : @YES};
    return AXIsProcessTrustedWithOptions((__bridge CFDictionaryRef)o) ? 1 : 0;
}

static int ax_frame(AXUIElementRef e, CGRect *r) {
    CFTypeRef p = NULL, s = NULL;
    int ok = 0;
    if (AXUIElementCopyAttributeValue(e, kAXPositionAttribute, &p) == kAXErrorSuccess &&
        AXUIElementCopyAttributeValue(e, kAXSizeAttribute, &s) == kAXErrorSuccess) {
        CGPoint pt;
        CGSize sz;
        if (AXValueGetValue((AXValueRef)p, kAXValueTypeCGPoint, &pt) && AXValueGetValue((AXValueRef)s, kAXValueTypeCGSize, &sz)) {
            *r = (CGRect){pt, sz};
            ok = 1;
        }
    }
    if (p) CFRelease(p);
    if (s) CFRelease(s);
    return ok;
}

static NSString *ax_role(AXUIElementRef e) {
    CFTypeRef v = NULL;
    if (AXUIElementCopyAttributeValue(e, kAXRoleAttribute, &v) != kAXErrorSuccess || !v) return nil;
    return (__bridge_transfer NSString *)v;
}

/// Parts of the window at `x,y,w,h` (points) of process `pid` worth taking
/// alone, like the page in a browser, a list or an editor: frames into `out`
/// as x, y, w, h. Nothing without the Accessibility permission; a slow app
/// gets 80 ms.
int bk_window_parts(int pid, double x, double y, double w, double h, double *out, int max) {
    if (!AXIsProcessTrusted()) return 0;
    @autoreleasepool {
        AXUIElementRef app = AXUIElementCreateApplication(pid);
        if (!app) return 0;
        AXUIElementSetMessagingTimeout(app, 0.05f);
        CFTypeRef wins = NULL;
        int n = 0;
        if (AXUIElementCopyAttributeValue(app, kAXWindowsAttribute, &wins) == kAXErrorSuccess && wins) {
            NSArray *list = (__bridge NSArray *)wins;
            id window = nil;
            for (id c in list) {
                CGRect r;
                if (ax_frame((__bridge AXUIElementRef)c, &r) && fabs(r.origin.x - x) < 2 && fabs(r.origin.y - y) < 2 &&
                    fabs(r.size.width - w) < 2 && fabs(r.size.height - h) < 2) {
                    window = c;
                    break;
                }
            }
            // panes worth a shot of their own, and what to look inside
            NSSet *parts = [NSSet setWithArray:@[ @"AXScrollArea", @"AXWebArea", @"AXTable", @"AXOutline", @"AXBrowser", @"AXTextArea", @"AXList" ]];
            NSSet *leaves = [NSSet setWithArray:@[ @"AXWebArea", @"AXTable", @"AXOutline", @"AXBrowser", @"AXTextArea", @"AXList" ]];
            NSMutableArray *queue = window ? [NSMutableArray arrayWithObject:@[ window, @0 ]] : [NSMutableArray array];
            CFAbsoluteTime until = CFAbsoluteTimeGetCurrent() + 0.08;
            int seen = 0;
            while (queue.count > 0 && n < max && seen < 400 && CFAbsoluteTimeGetCurrent() < until) {
                NSArray *item = queue.firstObject;
                [queue removeObjectAtIndex:0];
                AXUIElementRef e = (__bridge AXUIElementRef)item[0];
                int depth = [item[1] intValue];
                seen++;
                NSString *role = ax_role(e);
                CGRect r;
                if (depth > 0 && role && [parts containsObject:role] && ax_frame(e, &r) && r.size.width >= 80 && r.size.height >= 48 &&
                    r.size.width * r.size.height < w * h * 0.96) {
                    out[n * 4] = r.origin.x;
                    out[n * 4 + 1] = r.origin.y;
                    out[n * 4 + 2] = r.size.width;
                    out[n * 4 + 3] = r.size.height;
                    n++;
                }
                if (depth >= 12 || (role && [leaves containsObject:role])) continue;
                CFTypeRef kids = NULL;
                if (AXUIElementCopyAttributeValue(e, kAXChildrenAttribute, &kids) == kAXErrorSuccess && kids) {
                    for (id k in (__bridge NSArray *)kids) [queue addObject:@[ k, @(depth + 1) ]];
                    CFRelease(kids);
                }
            }
            CFRelease(wins);
        }
        CFRelease(app);
        return n;
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
