import json
import sys
from logging_setup import setup_logging

logger = setup_logging("cairo.replace_object_in_json")

def replace_object_in_json(file_path, key_to_replace, new_object):
    """
    Parses a JSON file, replaces an object by a specific key, and writes it back.

    Args:
        file_path (str): Path to the JSON file.
        key_to_replace (str): Key of the object to be replaced.
        new_object (dict): The custom object to replace the original object with.
    """
    try:
        # Read the JSON file
        with open(file_path, 'r') as file:
            data = json.load(file)

        def replace_single(obj):
            if isinstance(obj, dict):
                if key_to_replace in obj:
                    obj[key_to_replace] = new_object
                    return
                for k, v in obj.items():
                    replace_single(v)

        replace_single(data)

        with open(file_path, 'w') as file:
            json.dump(data, file, indent=4)

    except Exception as e:
        logger.exception("An error occurred: %s", e)

# if len(sys.argv) != 4:
#     sys.exit("Usage: replace_object_in_json.py json_file key new_object")
# json_file_path = sys.argv[1]
# key = sys.argv[2]
# new_object = sys.argv[3]

# replace_object_in_json(json_file_path, key, new_object)

# # Example usage
# if __name__ == "__main__":
#     # Replace this with the path to your JSON file
#     json_file_path = "data.json"

#     # The key to replace
#     key = "foo"

#     # The new custom object
#     new_object = {"customKey": "customValue"}

#     replace_object_in_json(json_file_path, key, new_object)
