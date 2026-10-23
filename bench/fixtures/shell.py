import subprocess

def run(cmd):
    return subprocess.run(cmd, shell=True)
