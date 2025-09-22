import logging
import os
from typing import Optional

_TRUE_VALUES = {"1", "true", "yes", "on", "debug", "trace"}


def _is_true(val: Optional[str]) -> bool:
    if val is None:
        return False
    return str(val).strip().lower() in _TRUE_VALUES


def is_debug_mode() -> bool:
    if _is_true(os.getenv("$DEBUG_MODE")):
        return True
    return False

def is_cairo_logs_enabled() -> bool:
    if _is_true(os.getenv("CAIRO_LOGS")):
        return True
    return False

def setup_logging(logger_name: Optional[str] = None) -> logging.Logger:
    # Configure root logger once
    root = logging.getLogger()
    if is_debug_mode():
        desired_level = logging.DEBUG
    elif is_cairo_logs_enabled():
        desired_level = logging.INFO
    else:
        desired_level = logging.WARNING

    if not root.handlers:
        handler = logging.StreamHandler()
        formatter = logging.Formatter("%(levelname)s: %(message)s")
        handler.setFormatter(formatter)
        root.addHandler(handler)

    root.setLevel(desired_level)

    # Return module-specific logger
    return logging.getLogger(logger_name) if logger_name else root
