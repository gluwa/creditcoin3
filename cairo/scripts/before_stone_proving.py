import json
import sys
import os
from run_executable import run_executable
from replace_object_in_json import replace_object_in_json
from logging_setup import setup_logging

logger = setup_logging("cairo.before_stone_proving")

def beforeStoneProving(inputPath, cairoLangDir, stoneProverDir):
    airProverConfig = cairoLangDir + "/cpu_air_prover_config.json"
    airParams = cairoLangDir + "/cpu_air_params.json"

    if not os.path.isdir(inputPath):
        logger.error(inputPath + " does not exist. This folder is supposed to hold the 'program_input.json'.")
        sys.exit(20)

    if not os.path.isfile(airParams):
        logger.error(airParams + " does not exist. This file needs to exist so the script can update parameters with accordance to execution trace.")
        sys.exit(22)

    if not os.path.isfile(airProverConfig):
        logger.error(airProverConfig + " does not exist.")
        sys.exit(23)

    publicInput = inputPath + "/public_input.json"
    traceFile = inputPath + "/trace.json"
    memoryFile = inputPath + "/memory.json"

    f = open(airParams)
    air_params = json.load(f)
    f.close()

    f = open(publicInput)
    public_input = json.load(f)
    f.close()

    last_layer_degree_bound = int(air_params["stark"]["fri"]["last_layer_degree_bound"])
    logger.info("last layer degree bound: %s", last_layer_degree_bound)

    fri_step_list = gen_fri_steps(public_input["n_steps"], last_layer_degree_bound)
    if fri_step_list == []:
        sys.exit("Too few execution steps")

    logger.info("generated FRI step list: %s", fri_step_list)
    fri = {
        "fri_step_list": fri_step_list,
        "last_layer_degree_bound": last_layer_degree_bound,
        "n_queries": 18,
        "proof_of_work_bits": 24,
    }

    replace_object_in_json(airParams, "fri", fri)

def gen_fri_steps(steps, last_layer_degree_bound):
    #  ∑fri_step_list = log₂(#steps) + 4 - log₂(last_layer_degree_bound)
    sigmaFriSteps = ((16 * steps) // last_layer_degree_bound).bit_length() - 1
    if sigmaFriSteps < 4: # too few steps
        return []

    # https://github.com/starkware-libs/stone-prover/issues/4?ref=blog.lambdaclass.com
    q, r = divmod(sigmaFriSteps - 4, 3)
    return [0, 4] + [3] * q + ([r] if r > 0 else [])
