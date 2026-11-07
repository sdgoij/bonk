package com.github.sdgoij.bonk

import uniffi.bonkc.benchArgon2

actual fun runArgon2Benchmark(): String {
    val report = benchArgon2()
    return "first=${report.firstMs} ms, second=${report.secondMs} ms, maxRSS=${report.maxRssKib} KiB"
}
