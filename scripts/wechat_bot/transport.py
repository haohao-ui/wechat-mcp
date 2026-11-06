"""Transport command builders for remote and local modes."""


def remote_argv(host: str, command: str) -> list:
    """Build the argument vector for executing a command either locally or via SSH.

    If host == "local", runs the command directly via /bin/sh -c.
    Otherwise runs the command via batch-mode SSH.
    """
    if host == 'local':
        return ['/bin/sh', '-c', command]
    return ['ssh', '-o', 'BatchMode=yes', '-o', 'LogLevel=ERROR', host, command]
