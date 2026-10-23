import sqlite3

def find(uid):
    conn = sqlite3.connect("app.db")
    return conn.execute("SELECT * FROM users WHERE id = " + uid)
