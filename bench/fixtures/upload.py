import os

def save(filename, data):
    path = os.path.join("/var/uploads", filename)
    with open(path, "wb") as f:
        f.write(data)
