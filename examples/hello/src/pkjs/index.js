// PKJS stub: sends one PING message to the watch when the JS runtime is
// ready. Used to verify the inbound AppMessage path end to end.
var keys = require('message_keys');

Pebble.addEventListener('ready', function () {
    console.log('pkjs ready, sending PING');
    var msg = {};
    msg[keys.PING] = 42;
    Pebble.sendAppMessage(
        msg,
        function () { console.log('pkjs PING sent'); },
        function (e) { console.log('pkjs PING failed: ' + JSON.stringify(e)); }
    );
});
