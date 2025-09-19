#!/usr/bin/env python

import json
import sys
import os
from run_executable import run_executable
from logging_setup import setup_logging

logger = setup_logging("cairo.run_cairo_program")

def runCairoProgram(source, inputPath, proofMode, layout):
    srcFname, _ = os.path.splitext(source)
    compiledFname = srcFname + "_compiled.json"

    programInputFile = inputPath + "/program_input.json"
    if not os.path.isfile(programInputFile):
        logger.error("%s does not exist. It is expected to contain the input data for cairo program.", programInputFile)
        sys.exit(25)
    logger.info("compiling %s...", source)
    if proofMode:
        result = run_executable("cairo-compile", [source, "--output", compiledFname, "--proof_mode"])
    else:
        result = run_executable("cairo-compile", [source, "--output", compiledFname])

    if result != 0:
        logger.error("compilation failed")
        sys.exit(30)
    if not os.path.isfile(compiledFname):
        logger.error("%s not generated.", compiledFname)
        sys.exit(31)

    privateInput = inputPath + "/private_input.json"
    publicInput = inputPath + "/public_input.json"
    traceFile = inputPath + "/trace.bin"
    memoryFile = inputPath + "/memory.bin"
    outputFile = inputPath + "/output.txt"

    logger.info("program: %s", compiledFname)
    logger.info("program input: %s", programInputFile)

    if proofMode:
        logger.info("air_private_input: %s", privateInput)
        logger.info("air_public_input: %s", publicInput)

        logger.info("cairo-running...")
        result = run_executable("cairo-run", ["--program=" + compiledFname, "--layout=" + layout, "--program_input=" + programInputFile, "--air_public_input=" + publicInput, "--air_private_input=" + privateInput, "--trace_file=" + traceFile, "--memory_file=" + memoryFile, "--print_output", "--proof_mode"], outputFile)
    else:
        logger.info("cairo-running...")
        result = run_executable("cairo-run", ["--program=" + compiledFname, "--layout=" + layout, "--program_input=" + programInputFile, "--trace_file=" + traceFile, "--memory_file=" + memoryFile, "--print_output"], outputFile)

    if result != 0:
        sys.exit(40)

if len(sys.argv) < 3:
    sys.exit("expected arguments: program_source_path input_path <proof_mode>")

sourcePath = sys.argv[1]
inputPath = sys.argv[2]
proofMode = len(sys.argv) > 3 and sys.argv[3] == "proof_mode"
layout = "recursive"

runCairoProgram(sourcePath, inputPath, proofMode, layout)
