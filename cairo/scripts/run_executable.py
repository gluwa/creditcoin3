import subprocess
from logging_setup import setup_logging, is_debug_mode, is_cairo_logs_enabled

logger = setup_logging("cairo.run_executable")

def run_executable(executable_path, args=None, outputFile=None):
    """
    Runs an executable and prints its output to stdout.

    Args:
        executable_path (str): Path to the executable file.
        args (list, optional): List of arguments to pass to the executable. Defaults to None.
    """
    if args is None:
        args = []

    try:
        # Run the executable with arguments
        result = subprocess.run(
            [executable_path] + args,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            check=True
        )

        if outputFile is not None:
            with open(outputFile, "w") as file:
                file.write(result.stdout)

        if is_debug_mode():
            logger.debug("stdout from %s:\n%s", executable_path, result.stdout)

        if is_cairo_logs_enabled():
            logger.info("stdout from %s:\n%s", executable_path, result.stdout)
        return 0

    except subprocess.CalledProcessError as e:
        logger.error("Error running %s (code %s)", executable_path, e.returncode)
        if is_debug_mode():
            logger.error("stdout:\n%s", e.stdout)
            logger.error("stderr:\n%s", e.stderr)

        if is_cairo_logs_enabled():
            logger.error("stdout:\n%s", e.stdout)
            logger.error("stderr:\n%s", e.stderr)
        return e.returncode
    except FileNotFoundError as e:
        logger.error("Executable not found: %s", executable_path)
        return e.errno
    except Exception as e:
        logger.exception("An unexpected error occurred: %s", e)
        return 42

# Example usage
if __name__ == "__main__":
    # Replace this with the path to your executable
    executable = "/path/to/your/executable"

    # Arguments to pass to the executable (if any)
    arguments = ["--example", "argument"]

    run_executable(executable, arguments)
