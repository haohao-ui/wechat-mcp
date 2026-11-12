import Foundation
import Darwin

@main
struct LifecycleTests {
    static func expect(_ condition: @autoclosure () -> Bool, _ message: String) {
        guard condition() else { fputs("FAIL: \(message)\n", stderr); exit(1) }
        print("PASS: \(message)")
    }
    static func until(_ predicate: () -> Bool) {
        let deadline = Date().addingTimeInterval(4)
        while !predicate() && Date() < deadline { usleep(10_000) }
        expect(predicate(), "termination observed")
    }
    static func tryStop(_ supervisor: CoreSupervisor, _ id: String) -> CoreMaintenanceStopResult {
        do { return try supervisor.stopForMaintenance(transactionID: id) }
        catch { expect(false, "unexpected stop error: \(error)"); return .notConfirmed }
    }
    static func completionFailureFixtures(root: URL) throws {
        for fault in ["sync", "open", "unlink", "restore-unwritable", "sync-once"] {
            let dir = root.appendingPathComponent("finish-" + fault)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
            let enabled = dir.appendingPathComponent("enabled")
            let marker = dir.appendingPathComponent("core.maintenance")
            func makeSupervisor() -> CoreSupervisor {
                CoreSupervisor(coreEnabledURL: enabled, binaryURL: URL(fileURLWithPath: "/usr/bin/true"), configURL: dir.appendingPathComponent("config"), logURL: dir.appendingPathComponent("log"), allowMissingConfigForTesting: true, gracePeriod: 0.05, pythonLoadedChecker: { false })
            }
            let subject = makeSupervisor()
            let id = UUID().uuidString
            try subject.startProcess(binaryPath: "/bin/sh", arguments: ["-c", "while :; do :; done"])
            try subject.beginMaintenance(transactionID: id)
            expect(tryStop(subject, id) == .stopped, "finish fault fixture has held exit evidence: " + fault)
            FileManager.default.createFile(atPath: enabled.path, contents: Data())
            var observedRemovedMarker = false
            if fault == "unlink" {
                subject.maintenanceMarkerUnlink = { _ in errno = EACCES; return -1 }
            } else if fault == "open" {
                subject.maintenanceDirectoryOpen = { _ in
                    observedRemovedMarker = observedRemovedMarker || !FileManager.default.fileExists(atPath: marker.path)
                    errno = EMFILE
                    return -1
                }
            } else {
                var calls = 0
                subject.maintenanceDirectorySync = { fd in
                    observedRemovedMarker = observedRemovedMarker || !FileManager.default.fileExists(atPath: marker.path)
                    calls += 1
                    if fault == "sync-once" && calls > 1 { return fsync(fd) }
                    if fault == "restore-unwritable" {
                        _ = chmod(dir.path, 0o500)
                    }
                    errno = EIO
                    return -1
                }
            }
            do {
                try subject.finishMaintenance(transactionID: id)
                expect(false, "injected completion failure must throw: " + fault)
            } catch CoreMaintenanceError.ioFailure(let code) {
                let expected = fault == "unlink" ? EACCES : (fault == "open" ? EMFILE : EIO)
                expect(code == expected, "completion preserves original IO error: " + fault)
            }
            if fault != "unlink" {
                expect(observedRemovedMarker, "injected directory failure occurs after marker unlink: " + fault)
            }
            // Restore test directory access, but never clear the supervisor's recovery latch.
            _ = chmod(dir.path, 0o700)
            expect(subject.maintenanceStatus == .recoveryRequired, "completion IO failure stays recoveryRequired: " + fault)
            expect(subject.maintenanceRecoveryPersistenceConfirmed == (fault == "unlink" || fault == "sync-once"), "restoration durability evidence matches injected outcome: " + fault)
            subject.reset()
            expect(subject.maintenanceStatus == .recoveryRequired, "reset cannot clear failed completion: " + fault)
            expect(subject.checkAndReconcile() == .keep, "failed completion suppresses enabled tick: " + fault)
            do { try subject.startProcess(arguments: []); expect(false, "failed completion blocks direct start") }
            catch { print("PASS: failed completion blocks direct start: " + fault) }
            // Removing the injected fault does not constitute recovery authorization.
            subject.maintenanceMarkerUnlink = { unlink($0) }
            subject.maintenanceDirectoryOpen = { open($0, O_RDONLY | O_NOFOLLOW) }
            subject.maintenanceDirectorySync = { fsync($0) }
            for transaction in [id, UUID().uuidString] {
                do { try subject.beginMaintenance(transactionID: transaction); expect(false, "failed completion blocks begin") }
                catch { print("PASS: failed completion blocks begin: " + fault) }
                do { try subject.finishMaintenance(transactionID: transaction); expect(false, "failed completion blocks finish") }
                catch { print("PASS: failed completion blocks finish: " + fault) }
            }
            let rebuilt = makeSupervisor()
            if fault == "restore-unwritable" {
                expect(!FileManager.default.fileExists(atPath: marker.path), "unwritable recovery cannot promise persisted marker")
                expect(rebuilt.maintenanceStatus == .inactive, "rebuild without marker has no durable evidence; caller journal recovery required")
            } else {
                expect(rebuilt.maintenanceStatus == .active(transactionID: id), "retained or restored marker survives rebuild: " + fault)
                expect(rebuilt.checkAndReconcile() == .keep, "rebuilt instance suppresses startup: " + fault)
                expect(tryStop(rebuilt, id) == .notConfirmed, "rebuilt instance still requires held process evidence: " + fault)
                do { try rebuilt.finishMaintenance(transactionID: id); expect(false, "rebuilt instance cannot finish on empty cache") }
                catch { print("PASS: rebuilt instance refuses unconfirmed finish: " + fault) }
                do { try rebuilt.beginMaintenance(transactionID: UUID().uuidString); expect(false, "rebuilt instance cannot replace interrupted transaction") }
                catch { print("PASS: rebuilt instance refuses different transaction: " + fault) }
            }
        }
    }
    static func main() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: root) }
        var policy = RestartPolicy()
        let now = Date()
        for delay in [5.0, 15.0, 60.0, 60.0] {
            expect(policy.recordExit(at: now) == .restart(after: delay), "real-crash backoff remains \(delay)s")
        }
        expect(policy.recordExit(at: now) == .giveUp, "real-crash threshold unchanged")
        // All supervisor paths explicit; no real core, launchctl, GUI, DB or HostLogger.
        let supervisor = CoreSupervisor(coreEnabledURL: root.appendingPathComponent("enabled"), binaryURL: URL(fileURLWithPath: "/bin/sh"), configURL: root.appendingPathComponent("config"), logURL: root.appendingPathComponent("core.log"), allowMissingConfigForTesting: true, gracePeriod: 0.1, pythonLoadedChecker: { false })
        func start(_ script: String) throws {
            try supervisor.startProcess(binaryPath: "/bin/sh", arguments: ["-c", script])
        }
        try start("exit 0")
        until { supervisor.lastExitCode != nil }
        expect(supervisor.currentStatus == .notRunning, "normal exit excluded from crash policy")
        supervisor.reset()
        for _ in 0..<6 {
            let ready = root.appendingPathComponent("ready")
            try? FileManager.default.removeItem(at: ready)
            try start("trap 'exit 0' TERM; touch '\(ready.path)'; while :; do :; done")
            until { FileManager.default.fileExists(atPath: ready.path) }
            if case .running(let pid) = supervisor.currentStatus { kill(pid, SIGTERM) }
            until { supervisor.currentStatus == .notRunning }
            expect(supervisor.lastExitCode == 0 && !supervisor.gaveUp && supervisor.restartingAt == nil, "direct TERM with handler exits normally")
        }
        supervisor.reset()
        try start("while :; do :; done")
        usleep(50_000)
        if case .running(let pid) = supervisor.currentStatus { kill(pid, SIGTERM) }
        until { supervisor.lastExitCode != nil }
        expect(supervisor.restartingAt != nil, "direct TERM signal counts as abnormal exit")
        supervisor.reset()
        try start("trap '' TERM; while :; do :; done")
        usleep(50_000)
        expect(supervisor.stopProcess() == "sigkill", "hung child requires timeout kill")
        until { supervisor.lastExitCode != nil }
        expect(!supervisor.gaveUp && supervisor.restartingAt == nil, "planned kill excluded from crash policy")
        supervisor.reset()
        try start("while :; do :; done")
        usleep(50_000)
        expect(supervisor.stopProcess(gracePeriod: 1) == "sigterm", "active stop confirms TERM exit")
        until { supervisor.lastExitCode != nil }
        expect(supervisor.restartingAt == nil, "active TERM excluded from crash policy")
        supervisor.reset()
        try start("exit 2")
        until { supervisor.lastExitCode != nil }
        expect(supervisor.currentStatus == .configError, "config error prevents restart")
        supervisor.reset()
        for index in 1...5 {
            try start("exit 1")
            until { if index == 5 { return supervisor.gaveUp }; return supervisor.restartingAt != nil }
        }
        expect(supervisor.currentStatus == .gaveUp, "five real crashes stop automatic restart")
        expect(decideCoreAction(coreEnabled: false, pythonLoaded: false, status: .notRunning) == .keep, "manual disabled state stays stopped")
        supervisor.reset()
        let marker = root.appendingPathComponent("core.maintenance")
        try Data(UUID().uuidString.utf8).write(to: marker)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: marker.path)
        FileManager.default.createFile(atPath: supervisor.coreEnabledURL.path, contents: Data())
        supervisor.binaryURL = URL(fileURLWithPath: "/usr/bin/true")
        expect(supervisor.checkAndReconcile() == .keep, "persisted maintenance blocks reconciliation after interruption")
        let id = try String(contentsOf: marker, encoding: .utf8)
        let other = UUID().uuidString
        func rejected(_ title: String, _ operation: () throws -> Void) {
            do { try operation(); expect(false, title) } catch { print("PASS: " + title) }
        }
        expect(supervisor.maintenanceStatus == .active(transactionID: id), "existing valid marker recognized")
        try supervisor.beginMaintenance(transactionID: id)
        rejected("different transaction cannot replace marker") { try supervisor.beginMaintenance(transactionID: other) }
        rejected("invalid transaction rejected") { try supervisor.beginMaintenance(transactionID: "") }
        rejected("oversized transaction rejected") { try supervisor.beginMaintenance(transactionID: String(repeating: "x", count: 4096)) }
        rejected("unknown owner cannot stop") { _ = try supervisor.stopForMaintenance(transactionID: other) }
        rejected("direct start blocked during maintenance") { try start("exit 0") }
        expect(tryStop(supervisor, id) == .notConfirmed, "empty cache is not exit confirmation")
        rejected("finish without held child rejected") { try supervisor.finishMaintenance(transactionID: id) }
        supervisor.reset()
        expect(supervisor.maintenanceStatus == .active(transactionID: id), "reset retains maintenance")
        let rebuilt = CoreSupervisor(coreEnabledURL: supervisor.coreEnabledURL, binaryURL: URL(fileURLWithPath: "/usr/bin/true"), configURL: supervisor.configURL, logURL: supervisor.logURL, allowMissingConfigForTesting: true, pythonLoadedChecker: { false })
        expect(rebuilt.checkAndReconcile() == .keep, "new supervisor retains interruption suppression")
        try Data("corrupt".utf8).write(to: marker)
        expect(supervisor.maintenanceStatus == .recoveryRequired, "corrupt marker fails closed")
        rejected("begin cannot overwrite corrupt marker") { try supervisor.beginMaintenance(transactionID: id) }
        expect(supervisor.checkAndReconcile() == .keep, "corrupt marker blocks tick")
        try FileManager.default.removeItem(at: marker)
        let target = root.appendingPathComponent("target")
        try Data(id.utf8).write(to: target)
        try FileManager.default.createSymbolicLink(at: marker, withDestinationURL: target)
        expect(supervisor.maintenanceStatus == .recoveryRequired, "symlink marker fails closed")
        rejected("begin cannot replace symlink") { try supervisor.beginMaintenance(transactionID: id) }
        try FileManager.default.removeItem(at: marker)
        let missingParent = CoreSupervisor(coreEnabledURL: root.appendingPathComponent("missing/enabled"), binaryURL: URL(fileURLWithPath: "/usr/bin/true"), configURL: supervisor.configURL, logURL: supervisor.logURL, allowMissingConfigForTesting: true, pythonLoadedChecker: { false })
        do {
            try missingParent.beginMaintenance(transactionID: id)
            expect(false, "missing directory must fail")
        } catch CoreMaintenanceError.ioFailure(let code) {
            expect(code == ENOENT, "missing directory reports actual IO errno")
        }
        try FileManager.default.setAttributes([.posixPermissions: 0o500], ofItemAtPath: root.path)
        do {
            try supervisor.beginMaintenance(transactionID: id)
            expect(false, "unwritable directory must fail")
        } catch CoreMaintenanceError.ioFailure(let code) {
            expect(code == EACCES, "write failure reports actual IO errno")
        }
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: root.path)
        let alias = root.appendingPathComponent("alias")
        try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: root)
        let linkedParent = CoreSupervisor(coreEnabledURL: alias.appendingPathComponent("enabled"), binaryURL: URL(fileURLWithPath: "/usr/bin/true"), configURL: supervisor.configURL, logURL: supervisor.logURL, allowMissingConfigForTesting: true, pythonLoadedChecker: { false })
        rejected("symlink parent cannot publish maintenance marker") { try linkedParent.beginMaintenance(transactionID: id) }
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: root.path)
        rejected("uncontrolled directory rejected") { try supervisor.beginMaintenance(transactionID: id) }
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: root.path)
        // Existing configuration-error and crash protections survive maintenance transitions.
        for script in ["exit 2", "exit 1"] {
            supervisor.reset()
            try start(script)
            until { supervisor.lastExitCode != nil }
            let configError = supervisor.isConfigError
            let retry = supervisor.restartingAt
            try supervisor.beginMaintenance(transactionID: id)
            try supervisor.beginMaintenance(transactionID: id)
            expect(supervisor.isConfigError == configError && supervisor.restartingAt == retry, "begin retains error/backoff evidence")
            try FileManager.default.removeItem(at: marker) // fixture recovery, not product API
        }
        supervisor.reset()
        try FileManager.default.removeItem(at: supervisor.coreEnabledURL)
        try start("trap '' TERM; while :; do :; done")
        usleep(50_000)
        if case .running(let pid) = supervisor.currentStatus { kill(pid, SIGSTOP) }
        try supervisor.beginMaintenance(transactionID: id)
        rejected("live child cannot finish maintenance") { try supervisor.finishMaintenance(transactionID: id) }
        let mode = try FileManager.default.attributesOfItem(atPath: marker.path)[.posixPermissions] as? NSNumber
        expect(mode?.intValue == 0o600, "published marker has mode 0600")
        expect(supervisor.checkAndReconcile() == .keep, "maintenance tick never stops or restarts implicitly")
        expect(tryStop(supervisor, id) == .stopped, "exact held child confirmed stopped")
        until { supervisor.lastExitCode != nil }
        rejected("wrong owner cannot finish") { try supervisor.finishMaintenance(transactionID: other) }
        try supervisor.finishMaintenance(transactionID: id)
        expect(supervisor.maintenanceStatus == .inactive, "owner completes maintenance")
        expect(supervisor.checkAndReconcile() == .keep, "finish preserves manual disabled flag")
        expect(supervisor.restartingAt == nil && !supervisor.gaveUp, "maintenance stop does not reset or consume crash policy")
        // Publication uses the same lock as start; race both entry points against begin.
        supervisor.reset()
        for _ in 0..<4 {
            try start("exit 1")
            until { supervisor.restartingAt != nil }
        }
        try start("while :; do :; done")
        try supervisor.beginMaintenance(transactionID: id)
        expect(tryStop(supervisor, id) == .stopped, "planned stop retains prior crash history")
        until { supervisor.lastExitCode != nil && supervisor.restartingAt == nil }
        try supervisor.finishMaintenance(transactionID: id)
        try start("exit 1")
        until { supervisor.gaveUp }
        expect(supervisor.currentStatus == .gaveUp, "fifth crash still gives up after maintenance")
        supervisor.reset()
        // All calls below occur after begin has returned, so none may launch a child.
        supervisor.binaryURL = URL(fileURLWithPath: "/usr/bin/true")
        FileManager.default.createFile(atPath: supervisor.coreEnabledURL.path, contents: Data())
        let group = DispatchGroup()
        let failures = NSLock()
        var lateStarts = 0
        try supervisor.beginMaintenance(transactionID: id)
        for _ in 0..<30 {
            group.enter()
            DispatchQueue.global().async {
                if supervisor.checkAndReconcile() != .keep { failures.lock(); lateStarts += 1; failures.unlock() }
                do { try supervisor.startProcess(arguments: []); failures.lock(); lateStarts += 1; failures.unlock() } catch {}
                group.leave()
            }
        }
        group.wait()
        expect(lateStarts == 0, "concurrent tick and direct starts blocked by persisted intent")
        let raceRoot = root.appendingPathComponent("race")
        try FileManager.default.createDirectory(at: raceRoot, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        let checked = DispatchSemaphore(value: 0)
        let resume = DispatchSemaphore(value: 0)
        let raceLog = raceRoot.appendingPathComponent("log")
        let race = CoreSupervisor(coreEnabledURL: raceRoot.appendingPathComponent("enabled"), binaryURL: URL(fileURLWithPath: "/usr/bin/true"), configURL: raceRoot.appendingPathComponent("config"), logURL: raceLog, allowMissingConfigForTesting: true, pythonLoadedChecker: {
            checked.signal()
            _ = resume.wait(timeout: .now() + 4)
            return false
        })
        FileManager.default.createFile(atPath: race.coreEnabledURL.path, contents: Data())
        group.enter()
        DispatchQueue.global().async { _ = race.checkAndReconcile(); group.leave() }
        expect(checked.wait(timeout: .now() + 4) == .success, "tick passed initial maintenance check")
        try race.beginMaintenance(transactionID: id)
        resume.signal()
        expect(group.wait(timeout: .now() + 4) == .success, "racing tick completes without deadlock")
        expect(!FileManager.default.fileExists(atPath: raceLog.path), "begin between tick check and start prevents launch")
        expect(race.currentStatus == .notRunning, "race leaves no child running")
        try completionFailureFixtures(root: root)


    }
}
