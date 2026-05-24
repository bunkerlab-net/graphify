// Objective-C with protocols, categories, properties, and class methods.
#import <Foundation/Foundation.h>

@protocol Drawable <NSObject>
- (void)draw;
@optional
- (CGFloat)opacity;
@end

@protocol Resizable
- (void)resizeTo:(CGFloat)scale;
@end

@interface Shape : NSObject <Drawable, Resizable>
@property (nonatomic, strong) NSString *name;
@property (nonatomic, assign) CGFloat width;
+ (instancetype)shapeWithName:(NSString *)name;
- (void)render;
@end

@implementation Shape

+ (instancetype)shapeWithName:(NSString *)name {
    Shape *s = [[Shape alloc] init];
    s.name = name;
    return s;
}

- (void)draw {
    NSLog(@"drawing %@", self.name);
}

- (CGFloat)opacity {
    return 1.0;
}

- (void)resizeTo:(CGFloat)scale {
    self.width *= scale;
}

- (void)render {
    [self draw];
}

@end

// Category
@interface Shape (Logging)
- (void)logState;
@end

@implementation Shape (Logging)
- (void)logState {
    NSLog(@"Shape %@ width=%f", self.name, self.width);
}
@end
