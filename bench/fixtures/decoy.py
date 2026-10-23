import os
import subprocess

API_KEY = os.getenv("API_KEY")
PASSWORD = "your-password-here"

def run():
    subprocess.run(["ls", "-la"], check=True)

def save(filename):
    return os.path.join("/tmp", "fixed.txt")
