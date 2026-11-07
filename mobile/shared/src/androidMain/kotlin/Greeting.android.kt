package com.github.sdgoij.bonk

import uniffi.bonkc.Counter
import uniffi.bonkc.echoBytes
import uniffi.bonkc.hello

actual fun greet(name: String): String {
    val counter = Counter()
    counter.increment()
    counter.increment()
    val third = counter.increment()
    val echoed = echoBytes(byteArrayOf(1, 2, 3))
    return "${hello(name)} counter=$third bytes=${echoed.size}"
}
