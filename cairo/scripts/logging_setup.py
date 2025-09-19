import logging
import os
from typing import Optional

_TRUE_VALUES = {"1", "true", "yes", "on", "debug", "trace"}


def _is_true(val: Optional[str]) -> bool:
    if val is None:
        return False
    return str(val).strip().lower() in _TRUE_VALUES


def is_debug_mode() -> bool:
    # Primary: the prover sets "$DEBUG_MODE" when --verbose is used (see prover/bin/prover.rs)
    if _is_true(os.getenv("$DEBUG_MODE")):
        return True

    # Fallbacks for robustness in other environments
    if _is_true(os.getenv("DEBUG_MODE")):
        return True
    if _is_true(os.getenv("PROVER_DEBUG")):
        return True

    # Level-style environment variables
    rust_log = os.getenv("RUST_LOG", "")
    if any(level in rust_log.lower() for level in ("debug", "trace")):
        return True

    log_level = os.getenv("LOG_LEVEL", "")
    if any(level in log_level.lower() for level in ("debug", "trace")):
        return True

    return False


def setup_logging(logger_name: Optional[str] = None) -> logging.Logger:
    # Configure root logger once
    root = logging.getLogger()
    desired_level = logging.DEBUG if is_debug_mode() else logging.WARNING

    if not root.handlers:
        handler = logging.StreamHandler()
        formatter = logging.Formatter("%(levelname)s: %(message)s")
        handler.setFormatter(formatter)
        root.addHandler(handler)

    root.setLevel(desired_level)

    # Return module-specific logger
    return logging.getLogger(logger_name) if logger_name else root
