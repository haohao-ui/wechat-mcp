import Foundation

/// 访问受保护文件夹（桌面、文稿、下载）时，系统权限弹窗会让 getxattr 等调用在内核里一直阻塞到有人点选。
/// worker 在 withDesktop 里做这些调用，阻塞就意味着桌面锁永远不释放、核心再也拿不到桌面（2026-11-08 实机）。
/// bounded 把操作放到后台线程，超时就抛出带文件夹名的错误；被卡住的后台线程会留在原地，但请求能返回、锁能释放。
struct FolderAccessBlocked: Error, CustomStringConvertible {
    let folder: String
    var description: String { "folder_access_blocked:\(folder)" }
}

private final class BoundedBox<T> {
    private let lock = NSLock()
    private var value: Result<T, Error>?
    func set(_ result: Result<T, Error>) { lock.lock(); value = result; lock.unlock() }
    func get() -> Result<T, Error>? { lock.lock(); defer { lock.unlock() }; return value }
}

func bounded<T>(seconds: Double, label: String, _ operation: @escaping () throws -> T) throws -> T {
    let done = DispatchSemaphore(value: 0)
    let box = BoundedBox<T>()
    DispatchQueue.global(qos: .userInitiated).async {
        box.set(Result { try operation() })
        done.signal()
    }
    guard done.wait(timeout: .now() + seconds) == .success, let result = box.get() else {
        throw FolderAccessBlocked(folder: label)
    }
    return try result.get()
}

/// 在动微信界面之前探测候选文件夹（各自限时）：
/// - 权限弹窗未处理（调用被挂起）→ 抛 folder_access_blocked:<文件夹>，不去点微信，也不长期占桌面锁；
/// - 已拒绝访问 → 记入 denied，不再在该文件夹里找文件；
/// - 文件夹不存在 → 跳过。
func preflightFolders(_ folders: [URL], seconds: Double = 3) throws -> (usable: [URL], denied: [String]) {
    var usable: [URL] = [], denied: [String] = []
    for folder in folders {
        do {
            _ = try bounded(seconds: seconds, label: folder.lastPathComponent) {
                try FileManager.default.contentsOfDirectory(atPath: folder.path)
            }
            usable.append(folder)
        } catch let blocked as FolderAccessBlocked {
            throw blocked
        } catch {
            let missing = (error as NSError).domain == NSCocoaErrorDomain
                && (error as NSError).code == NSFileReadNoSuchFileError
            if !missing { denied.append(folder.lastPathComponent) }
        }
    }
    return (usable, denied)
}
