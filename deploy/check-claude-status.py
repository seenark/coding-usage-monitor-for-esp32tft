#!/usr/bin/env python3
"""Accept Claude.ai CLI auth without emitting account metadata or response text."""
import json
import os
from pathlib import Path
import sys


def subscription_login(status):
    return (
        isinstance(status, dict)
        and status.get("authMethod") == "claude.ai"
        and status.get("configDirectory") == os.environ["CLAUDE_CONFIG_DIR"]
    )


if __name__ == "__main__":
    try:
        with Path(sys.argv[1]).open() as stream:
            valid = subscription_login(json.load(stream))
        sys.exit(0 if valid else 1)
    except (OSError, ValueError, KeyError, IndexError):
        sys.exit(1)
