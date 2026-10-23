from Crypto.Cipher import AES

def enc(key, data):
    return AES.new(key, AES.MODE_ECB)
