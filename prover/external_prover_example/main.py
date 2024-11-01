from flask import Flask, request, jsonify, send_file
import uuid
import os
import subprocess
import logging
import json
from concurrent.futures import ThreadPoolExecutor

app = Flask(__name__)

# In-memory storage to simulate work orders
work_orders = {}

# Directory to store uploaded files
UPLOAD_FOLDER = "/var/tmp/creditcoin3/claim-proofs/"
os.makedirs(UPLOAD_FOLDER, exist_ok=True)

# Thread pool executor for background tasks
executor = ThreadPoolExecutor()

def execute_script(query_id, work_order_dir):
    script_path = "./stone_prove_claim.sh"  # Replace with the correct path if needed

    # Execute the script and pass the work order directory as an argument
    process = subprocess.Popen(
        [script_path, work_order_dir],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,  # Ensures output is in text mode (str)
    )

    # Check if stdout is not None before iterating
    if process.stdout:
        for line in process.stdout:
            print(line, end="")  # Print each line of output in real-time

    # Wait for the process to complete and capture any error messages
    _, stderr = process.communicate()

    if stderr:
        logging.error(f"Error: {stderr}")
        work_orders[query_id]["status"] = "FAILED"
    else:
        logging.info("Work order completed successfully")
        work_orders[query_id]["status"] = "DONE"
        # Store the path to the proof file
        work_orders[query_id]["proof_path"] = os.path.join(work_order_dir, "proof.json")


@app.route('/api/prove', methods=['POST'])
def post_work_order():
    # Extract query ID from the form
    query_id = request.form.get("query_id")
    if not query_id:
        return jsonify({"error": "query_id is required"}), 400

    # Check for files in the multipart request
    files = request.files
    if not files:
        return jsonify({"error": "No files provided"}), 400

    # Generate a unique work order ID and create a directory for it
    work_order_dir = os.path.join(UPLOAD_FOLDER, query_id)
    logging.info(f"work_order_dir: {work_order_dir}")
    os.makedirs(work_order_dir, exist_ok=True)

    # Log and save each file
    for filename, file in files.items():
        file_path = os.path.join(work_order_dir, filename)

        # Handle private_input.json modifications
        if filename == "private_input.json":
            data = json.load(file)
            data["trace_path"] = os.path.join(work_order_dir, "trace.json")
            data["memory_path"] = os.path.join(work_order_dir, "memory.json")

            # Save modified private_input.json directly
            with open(file_path, 'w') as modified_file:
                json.dump(data, modified_file)
        else:
            # Save other files without modification
            file.save(file_path)

    # Simulate processing the files and set the initial work order status
    work_orders[query_id] = {
        "status": "PENDING",
        "proof_path": None  # No result yet
    }

    # Run the script asynchronously
    executor.submit(execute_script, query_id, work_order_dir)

    # Return the initial response
    response = {
        "query_id": query_id,
        "status": "PENDING"
    }
    return jsonify(response), 201

@app.route('/api/prove/<query_id>', methods=['GET'])
def get_work_order_status(query_id):
    # Check if the work order exists
    if query_id in work_orders:
        work_order_status = work_orders[query_id]["status"]
        response = {
            "query_id": query_id,
            "status": work_order_status
        }
        return jsonify(response), 200
    else:
        return jsonify({"error": "Work order not found"}), 404

@app.route('/api/prove/<query_id>/result', methods=['GET'])
def get_work_order_result(query_id):
    # Check if the work order exists and has completed processing
    if query_id in work_orders:
        work_order = work_orders[query_id]
        if work_order["status"] == "DONE" and work_order["proof_path"]:
            # Send the proof file as binary data
            return send_file(
                work_order["proof_path"],
                mimetype="application/octet-stream",
                as_attachment=True,
                download_name="proof.json"
            )
        elif work_order["status"] == "PENDING":
            return jsonify({"query_id": query_id, "status": "PENDING"}), 200
        else:
            return jsonify({"query_id": query_id, "status": "FAILED"}), 200
    else:
        return jsonify({"error": "Work order not found"}), 404

if __name__ == '__main__':
    logging.basicConfig(level=logging.INFO)
    app.run(debug=True)
