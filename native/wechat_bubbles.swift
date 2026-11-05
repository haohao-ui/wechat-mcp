// Conservative visual candidates for a calibrated dark-theme WeChat test window.
import Foundation
import CoreGraphics
import ImageIO
import Vision
import CryptoKit

enum Failure: Error { case invalid(String) }
struct Box { let x: Int; let y: Int; let width: Int; let height: Int }
struct Text { let body: String; let x: Double; let y: Double }

func recognize(_ image: CGImage, timeout: Double = 12) throws -> [Text] {
    let completed = DispatchSemaphore(value:0)
    var result: [Text] = []
    var failure: Error?
    DispatchQueue.global(qos:.userInitiated).async {
        do { result = try recognizeNow(image) } catch { failure = error }
        completed.signal()
    }
    guard completed.wait(timeout:.now()+timeout) == .success else { throw Failure.invalid("vision_timeout") }
    if let failure = failure { throw failure }
    return result
}

/// Loads the Vision text model once (cold start is ~30s on macOS 26+; warm frames take <1s).
func warmUpRecognizer() {
    guard let context = CGContext(data:nil,width:200,height:60,bitsPerComponent:8,bytesPerRow:0,
        space:CGColorSpaceCreateDeviceRGB(),bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue) else { return }
    context.setFillColor(CGColor(red:1,green:1,blue:1,alpha:1)); context.fill(CGRect(x:0,y:0,width:200,height:60))
    guard let image = context.makeImage() else { return }
    do { _ = try recognize(image, timeout:60) } catch { fputs("vision warm-up failed: \(error)\n", stderr) }
}

func recognizeNow(_ image: CGImage) throws -> [Text] {
    let request = VNRecognizeTextRequest()
    request.recognitionLanguages = ["zh-Hans", "zh-Hant", "en-US"]
    request.recognitionLevel = .accurate
    request.usesLanguageCorrection = true
    try VNImageRequestHandler(cgImage:image, options:[:]).perform([request])
    return (request.results ?? []).compactMap { item in
        guard let text = item.topCandidates(1).first else { return nil }
        return Text(body:text.string, x:item.boundingBox.midX*Double(image.width),
                    y:(1-item.boundingBox.midY)*Double(image.height))
    }.sorted { $0.y < $1.y }
}

func analyzeFrame(_ full: CGImage, window: [String:Any], mediaRoot: URL? = nil, recipient: String? = nil) throws -> [String:Any] {
    guard let pointWidth = window["width"] as? Double, let pointHeight = window["height"] as? Double,
          window["on_screen"] as? Bool == true,
          full.width == 880, full.height == 640,
          Double(full.width) == pointWidth.rounded(), Double(full.height) == pointHeight.rounded(),
          let title = full.cropping(to:CGRect(x:250,y:0,width:full.width-260,height:50)),
          // Ends at the input separator (y=496): a fully scrolled last bubble keeps ~8px above it.
          let chat = full.cropping(to:CGRect(x:315,y:60,width:full.width-325,height:full.height-204)) else {
        throw Failure.invalid("unsupported_image_geometry")
    }
    let titleNames = try recognize(title).filter { $0.x >= 50 && $0.x < 220 }.map(\.body)
    if let expected = recipient, titleNames != [expected] { throw Failure.invalid("allowlisted_title_mismatch") }
    let width = chat.width, height = chat.height
    var pixels = [UInt8](repeating:0,count:width*height*4)
    let rendered = pixels.withUnsafeMutableBytes { buffer -> Bool in
        guard let context = CGContext(data:buffer.baseAddress,width:width,height:height,
            bitsPerComponent:8,bytesPerRow:width*4,space:CGColorSpaceCreateDeviceRGB(),
            bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue) else { return false }
        context.draw(chat,in:CGRect(x:0,y:0,width:width,height:height)); return true
    }
    guard rendered else { throw Failure.invalid("bitmap_failed") }
    func color(_ index: Int) -> Int {
        let r=Int(pixels[index*4]), g=Int(pixels[index*4+1]), b=Int(pixels[index*4+2])
        if g > 110 && g > r+45 && g > b+30 { return 2 }
        if r >= 38 && r <= 82 && abs(r-g) <= 8 && abs(r-b) <= 8 { return 1 }
        return 0
    }
    // Refuse light/unknown themes rather than interpreting arbitrary rectangles as messages.
    let background = stride(from:0,to:width*height,by:101).filter {
        let r=Int(pixels[$0*4]),g=Int(pixels[$0*4+1]),b=Int(pixels[$0*4+2])
        return r < 37 && g < 37 && b < 37
    }.count
    guard background > (width*height/101)/2 else { throw Failure.invalid("uncalibrated_theme") }
    var visited = [Bool](repeating:false,count:width*height)
    var regions: [(Box,Int)] = []
    var clippedBubble = false
    for start in 0..<(width*height) where !visited[start] {
        visited[start] = true
        let kind = color(start)
        if kind == 0 { continue }
        var queue = [start], head = 0
        var left=start%width,right=left,top=start/width,bottom=top
        while head < queue.count {
            let index=queue[head]; head += 1
            let x=index%width,y=index/width
            left=min(left,x); right=max(right,x); top=min(top,y); bottom=max(bottom,y)
            for (nx,ny) in [(x-1,y),(x+1,y),(x,y-1),(x,y+1)] where nx >= 0 && nx < width && ny >= 0 && ny < height {
                let next=ny*width+nx
                if !visited[next] && color(next) == kind { visited[next]=true; queue.append(next) }
            }
        }
        let box=Box(x:left,y:top,width:right-left+1,height:bottom-top+1)
        guard queue.count >= 500, box.width >= 30, box.height >= 25, box.height <= 200 else { continue }
        let anchored = (kind == 1 && (42...72).contains(box.x)) ||
                       (kind == 2 && (45...80).contains(width-box.x-box.width))
        if anchored && box.y+box.height >= height-2 { clippedBubble = true }
        guard box.y >= 2, box.y+box.height < height-2 else { continue }
        if kind == 1 && (42...72).contains(box.x) { regions.append((box,kind)) }
        if kind == 2 && (45...80).contains(width-box.x-box.width) { regions.append((box,kind)) }
    }
    let texts = try recognize(chat)
    var bubbles: [[String:Any]] = []
    for (box,kind) in regions.sorted(by:{ $0.0.y < $1.0.y }) {
        let body=texts.filter { $0.x >= Double(box.x) && $0.x < Double(box.x+box.width) &&
                              $0.y >= Double(box.y) && $0.y < Double(box.y+box.height) }.map(\.body).joined(separator:"\n")
        if body.isEmpty { continue }
        var bytes = Data()
        for y in box.y..<(box.y+box.height) {
            bytes.append(contentsOf:pixels[(y*width+box.x)*4..<(y*width+box.x+box.width)*4])
        }
        let digest=SHA256.hash(data:bytes).map { String(format:"%02x",$0) }.joined()
        bubbles.append(["text":body,"direction":kind == 1 ? "inbound_visual" : "outbound_visual",
                        "digest":digest,"kind":"text","x":box.x,"y":box.y,"width":box.width,"height":box.height])
    }
    // Media rectangles have the same left anchor, but do not have a uniform text-bubble fill.
    var broadVisited=[Bool](repeating:false,count:width*height)
    func foreground(_ index: Int) -> Bool {
        return max(pixels[index*4],max(pixels[index*4+1],pixels[index*4+2])) > 37
    }
    for start in 0..<(width*height) where !broadVisited[start] {
        broadVisited[start]=true
        if !foreground(start) { continue }
        var queue=[start],head=0,left=start%width,right=left,top=start/width,bottom=top
        while head<queue.count {
            let index=queue[head];head+=1
            let x=index%width,y=index/width
            left=min(left,x);right=max(right,x);top=min(top,y);bottom=max(bottom,y)
            for (nx,ny) in [(x-1,y),(x+1,y),(x,y-1),(x,y+1)] where nx>=0 && nx<width && ny>=0 && ny<height {
                let next=ny*width+nx
                if !broadVisited[next] && foreground(next) { broadVisited[next]=true;queue.append(next) }
            }
        }
        let box=Box(x:left,y:top,width:right-left+1,height:bottom-top+1)
        guard (42...72).contains(box.x),box.width>=80,box.height>=55,queue.count>=2000 else { continue }
        if box.y+box.height>=height-2 { clippedBubble=true;continue }
        guard box.y>=2 else { continue }
        let body=texts.filter { $0.x>=Double(box.x) && $0.x<Double(box.x+box.width) && $0.y>=Double(box.y) && $0.y<Double(box.y+box.height) }.map(\.body).joined(separator:"\n")
        let names=body.components(separatedBy:"\n").filter {
            $0.range(of:#"\.(pdf|txt|md|csv|json|docx|xlsx|png|jpe?g)$"#,options:[.regularExpression,.caseInsensitive]) != nil
        }
        let isFile=names.count==1 && body.range(of:#"\d+(\.\d+)?\s*(KB|MB|GB|字节|B)"#,options:[.regularExpression,.caseInsensitive]) != nil
        let neutral=queue.filter { color($0)==1 }.count
        let isImage = !isFile && (box.height>200 || Double(neutral)/Double(box.width*box.height)<0.60)
        guard isFile || isImage else { continue }
        var bytes=Data()
        for y in box.y..<(box.y+box.height) { bytes.append(contentsOf:pixels[(y*width+box.x)*4..<(y*width+box.x+box.width)*4]) }
        let digest=SHA256.hash(data:bytes).map { String(format:"%02x",$0) }.joined()
        bubbles.removeAll { old in
            guard let x=old["x"] as? Int,let y=old["y"] as? Int else { return false }
            return abs(x-box.x)<10 && abs(y-box.y)<10
        }
        var item:[String:Any] = ["text":isImage ? "[图片]" : body,"direction":"inbound_visual","digest":digest,
            "kind":isImage ? "image" : "file","media_id":digest,"x":box.x,"y":box.y,"width":box.width,"height":box.height]
        if isFile {
            item["filename"]=names[0]
            let sizes=body.components(separatedBy:"\n").filter { $0.range(of:#"\d+(\.\d+)?\s*(KB|MB|GB|字节|B)"#,options:[.regularExpression,.caseInsensitive]) != nil }
            let identity="file:"+names[0]+":"+(sizes.last ?? "")
            item["digest"]=SHA256.hash(data:Data(identity.utf8)).map { String(format:"%02x",$0) }.joined()
        }
        if isImage,let directory=mediaRoot {
            let path=directory.appendingPathComponent(digest+".png")
            if !FileManager.default.fileExists(atPath:path.path) {
                guard let cropped=chat.cropping(to:CGRect(x:box.x,y:box.y,width:box.width,height:box.height)),
                      let destination=CGImageDestinationCreateWithURL(path as CFURL,"public.png" as CFString,1,nil) else { throw Failure.invalid("media_crop_failed") }
                CGImageDestinationAddImage(destination,cropped,nil)
                guard CGImageDestinationFinalize(destination) else { throw Failure.invalid("media_write_failed") }
                try FileManager.default.setAttributes([.posixPermissions:0o600],ofItemAtPath:path.path)
            }
            item["image_source"]="chat_thumbnail"
        }
        bubbles.append(item)
    }
    bubbles.sort { ($0["y"] as! Int)<($1["y"] as! Int) }
    let result: [String:Any] = ["profile":"wechat-4-dark-1x","title_lines":titleNames,
        "bubbles":bubbles,"platform_message_verified":false,"sender_verified":false,
        "width":full.width,"height":full.height,"clipped_bubble":clippedBubble]
    return result
}

#if !BOT_WORKER
@main
struct BubbleCLI {
    static func main() {
        do {
            guard CommandLine.arguments.count == 3,
                  let windowData = CommandLine.arguments[2].data(using:.utf8),
                  let window = try JSONSerialization.jsonObject(with:windowData) as? [String:Any],
                  let source = CGImageSourceCreateWithURL(URL(fileURLWithPath:CommandLine.arguments[1]) as CFURL,nil),
                  let full = CGImageSourceCreateImageAtIndex(source,0,nil) else { throw Failure.invalid("invalid_input") }
            warmUpRecognizer()
            let result = try analyzeFrame(full,window:window)
            let data = try JSONSerialization.data(withJSONObject:result,options:[.sortedKeys])
            print(String(decoding:data,as:UTF8.self))
        } catch { fputs("wechat_bubbles: \(error)\n",stderr); exit(1) }
    }
}
#endif
