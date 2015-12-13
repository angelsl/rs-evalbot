#!/usr/bin/env node

var cluster = require('cluster');
var util = require('util');
var vm = require('vm');

if (cluster.isMaster) {
    var data = new Buffer(0);
    
    cluster.setupMaster({
        exec: __filename,
        silent: true,
        args: []
    });
    var worker = cluster.fork();
    var outbuf = new Buffer(5);
    
    worker.on('message', function(message) {
        outbuf.writeUInt8(message.success ? 1 : 0, 0);
        outbuf.writeInt32LE(Buffer.byteLength(message.output, 'utf8'), 1);
        process.stdout.write(outbuf);
        process.stdout.write(message.output, 'utf8');
    });
    
    process.stdin.on('data', function(chunk) {
        data = Buffer.concat([data, chunk]);
        var len = 0;
        if (data.length >= 8 && data.length >= 8 + (len = data.readInt32LE(4))) {
            worker.send({
                timeout: data.readInt32LE(0),
                code: data.toString('utf8', 8, 8 + len)
            });
            data = new Buffer(0);
        }
    });
} else {
    var context = vm.createContext({
        console: console,
        module: module,
        process: process,
        require: require
    });
    
    var stdout;
    var buf = "";
    var callback = function(data) {
        stdout += data;
    };
    process.stdout.write = (function(write) {
        return function(string, encoding, fd) {
            callback.call(callback, string);
        };
    }(process.stdout.write));
    process.stderr.write = (function(write) {
        return function(string, encoding, fd) {
            callback.call(callback, string);
        };
    }(process.stderr.write));
    
    process.on('message', function(message) {
        var finished = true;
        buf += message.code;
        stdout = "";
        try {
            var out = vm.runInContext(buf, context, {
                filename: 'stdin',
                timeout: message.timeout
            });
            if (typeof out !== "undefined") {
                stdout += util.inspect(out);
            }
        } catch(err) {
            // FIXME hack hack hack
            if (err.name === "SyntaxError" && err.message === "Unexpected end of input") {
                finished = false;
            } else {
                stdout += err.toString();
            }
        }
        if (finished) {
            buf = "";
        }
        process.send({
            output: finished ? stdout : "(continue...)",
            success: finished
        });
    });
}