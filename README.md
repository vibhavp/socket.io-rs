Sending binary data

The client library for socket.io allows the browser to send binary data
with ordinary JSON data, treating it as if it was a normal JSON object
(with Buffer, ArrayBuffer
and Blob). This is done by replacing each binary object in the JSON data
with a "placeholder" object: `{"_placeholder": true, num: 0}`, with `num` being the
"attachment" number of the respective binary object. Depending on the transport used
the binary data is sent in later packets, as an engine.io "message".
For instance, using so.emit("foo", new Blob([1, 2, 3])) creates a BinaryEvent
packet with the following data:
["foo", {"_placeholder": true, num: 0}]. The blob is sent in a later packet.

TODO:

find out how non-JSON (text) strings are emitted