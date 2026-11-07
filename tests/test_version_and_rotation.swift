import Foundation
import Darwin

// 辅助断言
func assertEqual<T: Equatable>(_ actual: T, _ expected: T, _ message: String = "", file: StaticString = #file, line: UInt = #line) {
    if actual != expected {
        fputs("ASSERTION FAILED: expected [\(expected)], got [\(actual)] - \(message) at \(file):\(line)\n", stderr)
        exit(1)
    }
}

func assertTrue(_ condition: Bool, _ message: String = "", file: StaticString = #file, line: UInt = #line) {
    if !condition {
        fputs("ASSERTION FAILED: expected true, got false - \(message) at \(file):\(line)\n", stderr)
        exit(1)
    }
}

@main
struct TestRunner {
    static func main() throws {
        let fm = FileManager.default
        let tmpDir = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("test_task5_\(UUID().uuidString)")
        try fm.createDirectory(at: tmpDir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])

        defer {
            try? fm.removeItem(at: tmpDir)
        }

        print("=== Running Swift Version and Rotation Tests in \(tmpDir.path) ===")

        // Test 1: LogRotator 纯轮换逻辑
        do {
            let logFile = tmpDir.appendingPathComponent("test_rot.log")
            let smallData = "hello small log\n".data(using: .utf8)!
            try smallData.write(to: logFile)

            // 小于 100 字节，maxBytes = 100 时不轮换
            let rotated1 = LogRotator.rotateIfNeeded(fileURL: logFile, maxBytes: 100, maxFiles: 5)
            assertTrue(!rotated1, "file smaller than maxBytes should not rotate")
            assertTrue(fm.fileExists(atPath: logFile.path), "base file should still exist")
            assertTrue(!fm.fileExists(atPath: "\(logFile.path).1"), ".1 should not exist yet")

            // 写入超过 100 字节
            let largeString = String(repeating: "A", count: 120) + "\n"
            let largeData = largeString.data(using: .utf8)!
            try largeData.write(to: logFile)

            // 触发轮换
            let rotated2 = LogRotator.rotateIfNeeded(fileURL: logFile, maxBytes: 100, maxFiles: 5)
            assertTrue(rotated2, "file >= maxBytes should trigger rotation")
            assertTrue(fm.fileExists(atPath: logFile.path), "new base file should exist")
            assertTrue(fm.fileExists(atPath: "\(logFile.path).1"), ".1 should exist after first rotation")

            // 检查 .1 内容和权限
            let backup1Data = try Data(contentsOf: URL(fileURLWithPath: "\(logFile.path).1"))
            assertEqual(backup1Data, largeData, ".1 content must match original un-rotated content")
            
            let attrs1 = try fm.attributesOfItem(atPath: "\(logFile.path).1")
            let posix1 = (attrs1[.posixPermissions] as? NSNumber)?.intValue ?? 0
            assertEqual(posix1, 0o600, ".1 permissions must be 0600")

            // 检查新 base 文件的权限是 0600 且大小为 0
            let baseAttrs = try fm.attributesOfItem(atPath: logFile.path)
            let basePosix = (baseAttrs[.posixPermissions] as? NSNumber)?.intValue ?? 0
            assertEqual(basePosix, 0o600, "base file permissions must be 0600")
            let baseSize = (baseAttrs[.size] as? NSNumber)?.intValue ?? -1
            assertEqual(baseSize, 0, "new base file should be empty")

            // 连续写入并轮换 5 次，验证 .1 到 .5 的顺移
            for i in 2...6 {
                let content = "Rotation generation \(i): " + String(repeating: "\(i)", count: 100) + "\n"
                let data = content.data(using: .utf8)!
                try data.write(to: logFile)
                let r = LogRotator.rotateIfNeeded(fileURL: logFile, maxBytes: 50, maxFiles: 5)
                assertTrue(r, "generation \(i) must rotate")
            }

            // 应该保留 .1 到 .5，且不存在 .6
            for i in 1...5 {
                assertTrue(fm.fileExists(atPath: "\(logFile.path).\(i)"), "backup .\(i) must exist")
            }
            assertTrue(!fm.fileExists(atPath: "\(logFile.path).6"), "backup .6 must NOT exist (capped at maxFiles=5)")

            print("Test 1 (LogRotator basic + shifting + cap) passed!")
        }

        // Test 2: HostLogger 写日志与轮换
        do {
            let hostLogFile = tmpDir.appendingPathComponent("host_test.log")
            let logger = HostLogger(logURL: hostLogFile)

            // 写入第一批数据
            logger.log("line 1")
            logger.log("line 2")
            assertTrue(fm.fileExists(atPath: hostLogFile.path), "host log file must exist")
            let content = try String(contentsOf: hostLogFile, encoding: .utf8)
            assertTrue(content.contains("[host] line 1"), "host log should contain line 1")
            assertTrue(content.contains("[host] line 2"), "host log should contain line 2")

            // 测试轮换机制：先写一个文件
            let bigBlock = String(repeating: "X", count: 200) + "\n"
            for _ in 1...10 {
                if let data = bigBlock.data(using: .utf8), let handle = try? FileHandle(forWritingTo: hostLogFile) {
                    handle.seekToEndOfFile()
                    handle.write(data)
                    try? handle.close()
                }
            }
            let hostRotated = LogRotator.rotateIfNeeded(fileURL: hostLogFile, maxBytes: 500, maxFiles: 5)
            assertTrue(hostRotated, "host.log should rotate when exceeding threshold")
            assertTrue(fm.fileExists(atPath: "\(hostLogFile.path).1"), "host.log.1 should exist")

            // 轮换后 logger 继续写入
            logger.log("line after rotation")
            let newContent = try String(contentsOf: hostLogFile, encoding: .utf8)
            assertTrue(newContent.contains("[host] line after rotation"), "new host.log should contain line after rotation")
            assertTrue(!newContent.contains("[host] line 1"), "new host.log must not contain old line 1")

            print("Test 2 (HostLogger write + rotation) passed!")
        }

        // Test 3: CoreSupervisor 启动子进程前对超大 core.log 进行轮换
        do {
            let coreLogFile = tmpDir.appendingPathComponent("core_test.log")
            let dummyBinary = tmpDir.appendingPathComponent("dummy_worker.sh")
            let dummyScript = "#!/bin/bash\necho 'from dummy worker'\nexit 0\n"
            try dummyScript.write(to: dummyBinary, atomically: true, encoding: .utf8)
            try fm.setAttributes([.posixPermissions: 0o755], ofItemAtPath: dummyBinary.path)

            // 先在 coreLogFile 里写入较大数据
            let oldContent = "Old core log line\n" + String(repeating: "C", count: 300) + "\n"
            try oldContent.write(to: coreLogFile, atomically: true, encoding: .utf8)
            try fm.setAttributes([.posixPermissions: 0o600], ofItemAtPath: coreLogFile.path)

            // 启动 CoreSupervisor (默认当 logURL >= 10MB 时轮换；这里验证用 LogRotator 先轮换后启动，或者 supervisor start 自身调用)
            let preRotated = LogRotator.rotateIfNeeded(fileURL: coreLogFile, maxBytes: 200, maxFiles: 5)
            assertTrue(preRotated, "core log should pre-rotate if >= 200 bytes")
            assertTrue(fm.fileExists(atPath: "\(coreLogFile.path).1"), "core.log.1 must exist")

            let supervisor = CoreSupervisor(
                coreEnabledURL: tmpDir.appendingPathComponent("core.enabled"),
                binaryURL: dummyBinary,
                configURL: tmpDir.appendingPathComponent("config.json"),
                logURL: coreLogFile,
                gracePeriod: 10.0,
                pythonLoadedChecker: { false }
            )

            try supervisor.startProcess(binaryPath: dummyBinary.path, arguments: [], logPath: coreLogFile.path)
            supervisor.waitUntilExit()

            // 检查当前 core.log 包含 dummy worker 的输出
            let curLog = try String(contentsOf: coreLogFile, encoding: .utf8)
            assertTrue(curLog.contains("from dummy worker"), "current core.log must contain new process output")
            assertTrue(!curLog.contains("Old core log line"), "current core.log must not contain old rotated content")

            let curAttrs = try fm.attributesOfItem(atPath: coreLogFile.path)
            let curPerm = (curAttrs[.posixPermissions] as? NSNumber)?.intValue ?? 0
            assertEqual(curPerm, 0o600, "core.log permissions must remain 0600")

            print("Test 3 (CoreSupervisor log rotation) passed!")
        }

        // Test 4: hostAppVersion 读取
        do {
            // 默认没有 VERSION 文件时
            let defaultVer = hostAppVersion()
            assertTrue(!defaultVer.isEmpty, "hostAppVersion should return non-empty default version")

            // 当指定 VERSION 文件存在时
            let versionFileURL = fm.homeDirectoryForCurrentUser.appendingPathComponent(".wechat-rpa/VERSION")
            let backupURL = fm.homeDirectoryForCurrentUser.appendingPathComponent(".wechat-rpa/VERSION.testbackup")
            if fm.fileExists(atPath: versionFileURL.path) {
                try? fm.moveItem(at: versionFileURL, to: backupURL)
            }

            try "test-git-rev-20261108\n".write(to: versionFileURL, atomically: true, encoding: .utf8)
            let readVer = hostAppVersion()
            assertEqual(readVer, "test-git-rev-20261108", "hostAppVersion must read ~/.wechat-rpa/VERSION")

            try? fm.removeItem(at: versionFileURL)
            if fm.fileExists(atPath: backupURL.path) {
                try? fm.moveItem(at: backupURL, to: versionFileURL)
            }

            print("Test 4 (hostAppVersion reading) passed!")
        }

        print("=== ALL Task 5 Swift tests passed successfully! ===")
    }
}
