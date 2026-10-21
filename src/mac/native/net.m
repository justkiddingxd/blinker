// HTTP requests through NSURLSession (system TLS, proxy settings and keychain).

#import <Foundation/Foundation.h>
#include "bk.h"

typedef void (*bk_progress_cb)(void *ctx, double fraction);

@interface BKUpload : NSObject <NSURLSessionTaskDelegate, NSURLSessionDataDelegate>
@property(nonatomic) bk_progress_cb progress;
@property(nonatomic) void *ctx;
@property(nonatomic, strong) NSMutableData *body;
@property(nonatomic, strong) dispatch_semaphore_t done;
@property(nonatomic, strong) NSError *error;
@property(nonatomic) NSInteger status;
@end

@implementation BKUpload
- (void)URLSession:(NSURLSession *)s
                        task:(NSURLSessionTask *)task
             didSendBodyData:(int64_t)sent
              totalBytesSent:(int64_t)total
    totalBytesExpectedToSend:(int64_t)expected {
    (void)s;
    (void)task;
    (void)sent;
    if (self.progress && expected > 0) self.progress(self.ctx, (double)total / (double)expected);
}
- (void)URLSession:(NSURLSession *)s
              dataTask:(NSURLSessionDataTask *)task
    didReceiveResponse:(NSURLResponse *)response
     completionHandler:(void (^)(NSURLSessionResponseDisposition))handler {
    (void)s;
    (void)task;
    if ([response isKindOfClass:NSHTTPURLResponse.class]) self.status = ((NSHTTPURLResponse *)response).statusCode;
    handler(NSURLSessionResponseAllow);
}
- (void)URLSession:(NSURLSession *)s dataTask:(NSURLSessionDataTask *)task didReceiveData:(NSData *)data {
    (void)s;
    (void)task;
    if (self.body.length < (1 << 20)) [self.body appendData:data];
}
- (void)URLSession:(NSURLSession *)s task:(NSURLSessionTask *)task didCompleteWithError:(NSError *)error {
    (void)s;
    (void)task;
    self.error = error;
    dispatch_semaphore_signal(self.done);
}
@end

/// Sends `method` (POST with `body`, or GET...). Returns 1 with `*status`
/// and the response in `*resp` (malloc'd UTF-8), or
/// 0 with the NSURLError code in `*err_code` and its text in `*resp`.
int bk_http_request(const char *method, const char *url, const char **keys, const char **values, int nheaders, const uint8_t *body, size_t len, bk_progress_cb progress,
                 void *ctx, int *status, char **resp, int *err_code) {
    @autoreleasepool {
        *resp = NULL;
        *err_code = 0;
        NSURL *u = [NSURL URLWithString:bk_str(url)];
        if (!u || !u.scheme || !u.host) {
            *err_code = -1000;
            return 0;
        }
        NSMutableURLRequest *req = [NSMutableURLRequest requestWithURL:u cachePolicy:NSURLRequestReloadIgnoringLocalCacheData timeoutInterval:120];
        req.HTTPMethod = bk_str(method);
        for (int i = 0; i < nheaders; i++) [req setValue:bk_str(values[i]) forHTTPHeaderField:bk_str(keys[i])];
        [req setValue:@"Blinker" forHTTPHeaderField:@"User-Agent"];
        BKUpload *up = [BKUpload new];
        up.progress = progress;
        up.ctx = ctx;
        up.body = [NSMutableData new];
        up.done = dispatch_semaphore_create(0);
        NSURLSessionConfiguration *cfg = NSURLSessionConfiguration.ephemeralSessionConfiguration;
        cfg.timeoutIntervalForRequest = 60;
        cfg.timeoutIntervalForResource = 600;
        NSURLSession *session = [NSURLSession sessionWithConfiguration:cfg delegate:up delegateQueue:nil];
        NSURLSessionTask *task;
        if (len > 0) {
            NSData *data = [NSData dataWithBytesNoCopy:(void *)body length:len freeWhenDone:NO];
            task = [session uploadTaskWithRequest:req fromData:data];
        } else {
            task = [session dataTaskWithRequest:req];
        }
        [task resume];
        dispatch_semaphore_wait(up.done, DISPATCH_TIME_FOREVER);
        [session finishTasksAndInvalidate];
        if (up.error) {
            *err_code = (int)up.error.code;
            *resp = strdup(up.error.localizedDescription.UTF8String ?: "");
            return 0;
        }
        *status = (int)up.status;
        NSString *text = [[NSString alloc] initWithData:up.body encoding:NSUTF8StringEncoding] ?: @"";
        *resp = strdup(text.UTF8String ?: "");
        return 1;
    }
}
