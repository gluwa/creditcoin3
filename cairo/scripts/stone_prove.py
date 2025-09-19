#!/usr/bin/env python

import json
import sys
import os
import datetime

from before_stone_proving import beforeStoneProving
from run_executable import run_executable
from logging_setup import setup_logging

logger = setup_logging("cairo.stone_prove")

def stoneProve(inputPath, cairoLangDir, stoneProverDir, force, generateAnnotations):
    proofFile = inputPath + "/proof.json"
    programInputFile = inputPath + "/program_input.json"
    privateInput = inputPath + "/private_input.json"
    publicInput = inputPath + "/public_input.json"

    airProverConfig = cairoLangDir + "/cpu_air_prover_config.json"
    airParams = cairoLangDir + "/cpu_air_params.json"


    if os.path.isfile(proofFile) and not force:
        logger.warning("%s already exists, skipping stone-proving. Use 'force' to or delete the proof file", proofFile)
        sys.exit(43)

    beforeStoneProving(inputPath, cairoLangDir, stoneProverDir)

    logger.info("out_file: %s", proofFile)
    logger.info("private_input_file: %s", privateInput)
    logger.info("public_input_file: %s", publicInput)
    logger.info("prover_config_file: %s", airProverConfig)
    logger.info("parameter_file: %s", airParams)
    logger.info("generating proof (will take a while)...")

    logger.info("started: %s", datetime.datetime.now().time())
    if generateAnnotations:
        run_executable(stoneProverDir + "/cpu_air_prover",
                       ["--out_file=" + proofFile, "--private_input_file=" + privateInput, "--public_input_file=" + publicInput, "--prover_config_file=" + airProverConfig, "--parameter_file=" + airParams, "-generate_annotations" ])
    else:
        run_executable(stoneProverDir + "/cpu_air_prover",
                       ["--out_file=" + proofFile, "--private_input_file=" + privateInput, "--public_input_file=" + publicInput, "--prover_config_file=" + airProverConfig, "--parameter_file=" + airParams])

    logger.info("finished: %s", datetime.datetime.now().time())

if len(sys.argv) < 2:
    sys.exit("expected arguments: input_path cairo_lang_dir stone_prover_dir <force> <generate_annotations>")

inputPath = sys.argv[1]
cairoLangDir = sys.argv[2]
stoneProverDir = sys.argv[3]
force = len(sys.argv) > 4 and sys.argv[4] == "force"
generateAnnotations = len(sys.argv) > 5 and sys.argv[5] == "generate_annotations"

stoneProve(inputPath, cairoLangDir, stoneProverDir, force, generateAnnotations)
