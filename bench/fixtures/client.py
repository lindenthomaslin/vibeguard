import requests

def fetch(url):
    return requests.get(url, verify=False)
