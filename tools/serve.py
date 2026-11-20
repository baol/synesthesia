#!/usr/bin/env python3
"""Serve a folder on 127.0.0.1 with HTTP Range requests, so the browser can
seek in audio and video (`python3 -m http.server` can't).

Usage: tools/serve.py PORT DIRECTORY
"""
import functools
import http.server
import os
import re
import sys


class Handler(http.server.SimpleHTTPRequestHandler):
    remaining = None  # bytes left to send of a range, or None for a whole file

    def send_head(self):
        self.remaining = None
        path = self.translate_path(self.path)
        match = re.fullmatch(r"bytes=(\d*)-(\d*)", self.headers.get("Range", ""))
        if not match or not os.path.isfile(path):
            return super().send_head()
        size = os.path.getsize(path)
        first, last = match.groups()
        if first:  # "bytes=N-" or "bytes=N-M"
            start, end = int(first), min(int(last or size - 1), size - 1)
        else:  # "bytes=-N": the last N bytes
            start, end = max(size - int(last or 0), 0), size - 1
        if start > end:
            self.send_response(416)
            self.send_header("Content-Range", f"bytes */{size}")
            self.end_headers()
            return None
        f = open(path, "rb")
        f.seek(start)
        self.send_response(206)
        self.send_header("Content-Type", self.guess_type(path))
        self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
        self.send_header("Content-Length", str(end - start + 1))
        self.end_headers()
        self.remaining = end - start + 1
        return f

    def copyfile(self, source, outputfile):
        try:
            if self.remaining is None:
                return super().copyfile(source, outputfile)
            while self.remaining > 0:
                chunk = source.read(min(1 << 16, self.remaining))
                if not chunk:
                    break
                outputfile.write(chunk)
                self.remaining -= len(chunk)
        except (BrokenPipeError, ConnectionResetError):
            pass  # the browser stopped listening (it does whenever it seeks)

    def end_headers(self):
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Cache-Control", "no-cache")
        super().end_headers()


if __name__ == "__main__":
    if len(sys.argv) != 3 or not sys.argv[1].isdigit():
        sys.exit(__doc__.strip())
    port, directory = int(sys.argv[1]), sys.argv[2]
    handler = functools.partial(Handler, directory=directory)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", port), handler)
    print(f"Serving {directory} at http://127.0.0.1:{port}/")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
