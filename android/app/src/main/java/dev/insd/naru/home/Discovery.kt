package dev.insd.naru.home

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.compose.runtime.mutableStateListOf
import dev.insd.naru.TAG

/** A Screen Sharing Mac found on the local network. Its address is resolved when picked. */
data class Found(val name: String, val service: NsdServiceInfo)

/**
 * Finds Screen Sharing Macs on the local network over Bonjour (`_rfb._tcp`).
 *
 * - [found] is Compose state. NSD callbacks arrive on another thread, so updates are posted to the main thread.
 * - Discovery runs only while the screen is visible, between [start] and [stop].
 */
class Discovery(context: Context) {
    private val manager = context.getSystemService(NsdManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private var listener: NsdManager.DiscoveryListener? = null

    val found = mutableStateListOf<Found>()

    fun start() {
        if (listener != null) {
            return
        }

        val next = object : NsdManager.DiscoveryListener {
            override fun onServiceFound(service: NsdServiceInfo) {
                main.post {
                    if (found.none { it.name == service.serviceName }) found += Found(service.serviceName, service)
                }
            }

            override fun onServiceLost(service: NsdServiceInfo) {
                main.post { found.removeAll { it.name == service.serviceName } }
            }

            override fun onDiscoveryStarted(serviceType: String) = Unit
            override fun onDiscoveryStopped(serviceType: String) = Unit

            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                Log.w(TAG, "Failed to start network discovery: $errorCode")
                main.post { if (listener === this) listener = null }
            }

            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) = Unit
        }

        listener = next
        manager.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, next)
    }

    fun stop() {
        val current = listener ?: return
        listener = null
        runCatching { manager.stopServiceDiscovery(current) }
        found.clear()
    }

    /** Resolves the found Mac's IPv4 address. The result arrives on the main thread through [done], or `null` on failure. */
    @Suppress("DEPRECATION")
    fun resolve(target: Found, done: (String?) -> Unit) {
        manager.resolveService(target.service, object : NsdManager.ResolveListener {
            override fun onServiceResolved(service: NsdServiceInfo) {
                val address = service.host?.hostAddress?.takeIf { '.' in it }
                main.post { done(address) }
            }

            override fun onResolveFailed(service: NsdServiceInfo, errorCode: Int) {
                Log.w(TAG, "Failed to resolve the address: $errorCode")
                main.post { done(null) }
            }
        })
    }

    companion object {
        private const val SERVICE_TYPE = "_rfb._tcp."
    }
}
