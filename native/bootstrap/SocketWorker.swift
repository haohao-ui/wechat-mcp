import Foundation
import Darwin

enum WorkerError: Error, LocalizedError {
    case unavailable(String)
    var errorDescription: String? {
        switch self { case .unavailable(let message): return message }
    }
}

/// The GUI application owns TCC permissions; SSH only transports MCP bytes.
final class SocketWorker {
    private let lock = NSLock()
    private var listener: Int32 = -1
    private var child: Process?
    private var socketPath: String?

    private func address(_ path: String) throws -> sockaddr_un {
        var value = sockaddr_un()
        value.sun_family = sa_family_t(AF_UNIX)
        value.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        let bytes = Array(path.utf8) + [0]
        guard bytes.count <= MemoryLayout.size(ofValue: value.sun_path) else {
            throw WorkerError.unavailable("本机连接路径过长")
        }
        withUnsafeMutableBytes(of: &value.sun_path) { destination in
            destination.copyBytes(from: bytes)
        }
        return value
    }

    func start() throws {
        lock.lock(); defer { lock.unlock() }
        if listener >= 0 { return }
        let directory = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".wechat-rpa", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true,
                                               attributes: [.posixPermissions: 0o700])
        let attributes = try FileManager.default.attributesOfItem(atPath: directory.path)
        guard (attributes[.ownerAccountID] as? NSNumber)?.uint32Value == geteuid(),
              (attributes[.posixPermissions] as? NSNumber)?.intValue == 0o700,
              attributes[.type] as? FileAttributeType == .typeDirectory else {
            throw WorkerError.unavailable("桌面服务目录必须仅当前用户可访问")
        }
        // Worker updates stay outside the sealed GUI bundle to preserve its identity.
        let helpers = directory.appendingPathComponent("bin", isDirectory: true)
        for name in ["wechat-mcp", "wechat_ocr", "wechat_click", "wechat_window"] {
            let binary = helpers.appendingPathComponent(name)
            let info = try FileManager.default.attributesOfItem(atPath: binary.path)
            guard info[.type] as? FileAttributeType == .typeRegular,
                  (info[.ownerAccountID] as? NSNumber)?.uint32Value == geteuid(),
                  ((info[.posixPermissions] as? NSNumber)?.intValue ?? 0o777) & 0o022 == 0,
                  FileManager.default.isExecutableFile(atPath: binary.path) else {
                throw WorkerError.unavailable("桌面服务程序缺失或权限不安全")
            }
        }
        let path = directory.appendingPathComponent("worker.sock").path
        var addr = try address(path)
        if FileManager.default.fileExists(atPath: path) {
            let old = try FileManager.default.attributesOfItem(atPath: path)
            guard old[.type] as? FileAttributeType == .typeSocket,
                  (old[.ownerAccountID] as? NSNumber)?.uint32Value == geteuid() else {
                throw WorkerError.unavailable("连接路径被其他文件占用")
            }
            let probe = socket(AF_UNIX, SOCK_STREAM, 0)
            guard probe >= 0 else { throw WorkerError.unavailable("无法检查已有连接") }
            let connected = withUnsafePointer(to: &addr) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    Darwin.connect(probe, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
                }
            }
            let reason = errno
            close(probe)
            guard connected != 0 && (reason == ECONNREFUSED || reason == ENOENT) else {
                throw WorkerError.unavailable("已有桌面服务正在使用连接路径")
            }
            try FileManager.default.removeItem(atPath: path)
        }
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw WorkerError.unavailable("无法建立本机连接") }
        guard fcntl(fd, F_SETFD, FD_CLOEXEC) == 0 else {
            close(fd)
            throw WorkerError.unavailable("无法隔离本机连接描述符")
        }
        let bound = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard bound == 0 else { close(fd); throw WorkerError.unavailable("本机连接绑定失败") }
        do {
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: path)
            guard listen(fd, 1) == 0 else { throw WorkerError.unavailable("本机连接监听失败") }
        } catch {
            close(fd)
            try? FileManager.default.removeItem(atPath: path)
            throw error
        }
        listener = fd
        socketPath = path
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            self?.acceptClients(fd: fd, helpers: helpers, directory: directory)
        }
    }

    private func acceptClients(fd: Int32, helpers: URL, directory: URL) {
        while true {
            lock.lock(); let running = listener == fd; lock.unlock()
            if !running { return }
            let client = accept(fd, nil, nil)
            if client < 0 { if errno == EINTR { continue }; return }
            guard fcntl(client, F_SETFD, FD_CLOEXEC) == 0 else { close(client); continue }
            var uid: uid_t = 0
            var gid: gid_t = 0
            guard getpeereid(client, &uid, &gid) == 0, uid == geteuid() else { close(client); continue }
            lock.lock()
            if child != nil { lock.unlock(); close(client); continue }
            let handle = FileHandle(fileDescriptor: client, closeOnDealloc: true)
            let process = Process()
            process.executableURL = helpers.appendingPathComponent("wechat-mcp")
            process.currentDirectoryURL = helpers
            var environment = ProcessInfo.processInfo.environment
            environment["MESSAGE_GATEWAY_DB"] = directory.appendingPathComponent("test.sqlite3").path
            environment["MESSAGE_GATEWAY_CONFIG"] = Bundle.main.bundleURL.appendingPathComponent("Contents/Resources/gateway.test.json").path
            environment["WECHAT_MCP_DESKTOP_RUNTIME"] = "legacy"
            process.environment = environment
            process.standardInput = handle
            process.standardOutput = handle
            process.standardError = FileHandle.standardError
            process.terminationHandler = { [weak self] finished in
                handle.closeFile()
                guard let self = self else { return }
                self.lock.lock()
                if self.child === finished { self.child = nil }
                self.lock.unlock()
            }
            do {
                try process.run()
                child = process
            } catch {
                handle.closeFile()
            }
            lock.unlock()
        }
    }

    func stop() {
        lock.lock()
        let fd = listener
        listener = -1
        let running = child
        child = nil
        let path = socketPath
        socketPath = nil
        lock.unlock()
        if fd >= 0 { shutdown(fd, SHUT_RDWR); close(fd) }
        if running?.isRunning == true { running?.terminate() }
        if let path = path { try? FileManager.default.removeItem(atPath: path) }
    }

    /// Exercise the real MCP server without requesting any desktop operations.
    func testTransport() throws -> Int {
        try start()
        defer { stop() }
        lock.lock(); let path = socketPath; lock.unlock()
        guard let path = path else { throw WorkerError.unavailable("测试连接未启动") }
        var addr = try address(path)
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw WorkerError.unavailable("测试客户端创建失败") }
        let handle = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
        defer { handle.closeFile() }
        let connected = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard connected == 0 else { throw WorkerError.unavailable("测试客户端连接失败") }
        func request(_ object: [String: Any]) throws -> [String: Any] {
            var bytes = try JSONSerialization.data(withJSONObject: object)
            bytes.append(10)
            handle.write(bytes)
            let deadline = ProcessInfo.processInfo.systemUptime + 10
            var reply = Data()
            while !reply.contains(10) {
                guard ProcessInfo.processInfo.systemUptime < deadline, reply.count < 1024 * 1024 else {
                    throw WorkerError.unavailable("MCP测试响应超时或超长")
                }
                var descriptor = pollfd(fd: fd, events: Int16(POLLIN), revents: 0)
                let ready = poll(&descriptor, 1, 100)
                if ready < 0 { if errno == EINTR { continue }; throw WorkerError.unavailable("MCP测试读取失败") }
                if ready == 0 { continue }
                var buffer = [UInt8](repeating: 0, count: 4096)
                let count = Darwin.read(fd, &buffer, buffer.count)
                guard count > 0 else { throw WorkerError.unavailable("MCP测试连接提前关闭") }
                reply.append(contentsOf: buffer.prefix(count))
            }
            guard let line = reply.split(separator: 10).first,
                  let value = try JSONSerialization.jsonObject(with: Data(line)) as? [String: Any],
                  value["id"] as? Int == object["id"] as? Int,
                  value["error"] == nil,
                  let result = value["result"] as? [String: Any] else {
                throw WorkerError.unavailable("MCP测试协议响应无效")
            }
            return result
        }
        _ = try request(["jsonrpc":"2.0", "id":1, "method":"initialize",
                         "params":["protocolVersion":"2025-06-18", "capabilities":[:], "clientInfo":["name":"desktop-host-self-test", "version":"1"]]])
        let tools = try request(["jsonrpc":"2.0", "id":2, "method":"tools/list", "params":[:]])
        guard let list = tools["tools"] as? [[String: Any]], !list.isEmpty else {
            throw WorkerError.unavailable("MCP测试未发现工具")
        }
        return list.count
    }
}
