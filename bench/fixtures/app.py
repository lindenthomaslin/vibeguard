import hashlib
import os
import pickle

API_KEY = "sk-live-9f8a7b6c5d4e3f2a1b"

def ping(host):
    os.system("ping -c 1 " + host)

def digest(pw):
    return hashlib.md5(pw.encode())

def load(blob):
    return pickle.loads(blob)

def calc(expr):
    return eval(expr)

DEBUG = True
