#include <linux/bpf.h>
#include <linux/in.h>
#include <linux/if_ether.h>
#include <linux/ip.h>
#include <linux/udp.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_endian.h>

struct rtp_tuple {
    __be32 src_ip;
    __be16 src_port;
};

struct relay_dest {
    __be32 dst_ip;
    __be16 dst_port;
};

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __type(key, struct rtp_tuple);
    __type(value, struct relay_dest);
    __uint(max_entries, 65536);
} rtp_relay_map SEC(".maps") = {};

SEC("xdp")
int rtp_relay_prog(struct xdp_md *ctx) {
    void *data_end = (void *)(long)ctx->data_end;
    void *data = (void *)(long)ctx->data;

    struct ethhdr *eth = data;
    if ((void *)(eth + 1) > data_end)
        return XDP_PASS;

    if (eth->h_proto != bpf_htons(ETH_P_IP))
        return XDP_PASS;

    struct iphdr *ip = (void *)(eth + 1);
    if ((void *)(ip + 1) > data_end)
        return XDP_PASS;

    if (ip->protocol != IPPROTO_UDP)
        return XDP_PASS;

    struct udphdr *udp = (void *)(ip + 1);
    if ((void *)(udp + 1) > data_end)
        return XDP_PASS;

    // 匹配 RTP 源端口和源 IP 
    struct rtp_tuple key = {
        .src_ip = ip->saddr,
        .src_port = udp->source
    };

    struct relay_dest *dest = bpf_map_lookup_elem(&rtp_relay_map, &key);
    if (!dest) {
        return XDP_PASS; // 未匹配到中继规则，上送用户态
    }

    // 增量改写校验和与目的 IP/Port
    __be32 old_daddr = ip->daddr;
    __be16 old_dport = udp->dest;

    ip->daddr = dest->dst_ip;
    udp->dest = dest->dst_port;

    // 交换以太网 MAC 地址 (为了发回发送方)
    unsigned char tmp_mac[ETH_ALEN];
    __builtin_memcpy(tmp_mac, eth->h_dest, ETH_ALEN);
    __builtin_memcpy(eth->h_dest, eth->h_source, ETH_ALEN);
    __builtin_memcpy(eth->h_source, tmp_mac, ETH_ALEN);

    // 快速增量改写 IP 校验和 (XDP_TX 不需要完整重算)
    __u32 csum = ~bpf_ntohs(ip->check);
    csum -= old_daddr & 0xFFFF;
    csum -= (old_daddr >> 16) & 0xFFFF;
    csum += ip->daddr & 0xFFFF;
    csum += (ip->daddr >> 16) & 0xFFFF;
    csum = (csum & 0xFFFF) + (csum >> 16);
    csum = (csum & 0xFFFF) + (csum >> 16);
    ip->check = bpf_htons(~csum);

    // 快速增量改写 UDP 校验和
    if (udp->check != 0) {
        __u32 udp_csum = ~bpf_ntohs(udp->check);
        // 改写 pseudo-header IP 和 UDP port 变动部分
        udp_csum -= old_daddr & 0xFFFF;
        udp_csum -= (old_daddr >> 16) & 0xFFFF;
        udp_csum += ip->daddr & 0xFFFF;
        udp_csum += (ip->daddr >> 16) & 0xFFFF;
        udp_csum -= bpf_ntohs(old_dport);
        udp_csum += bpf_ntohs(udp->dest);
        udp_csum = (udp_csum & 0xFFFF) + (udp_csum >> 16);
        udp_csum = (udp_csum & 0xFFFF) + (udp_csum >> 16);
        udp->check = bpf_htons(~udp_csum);
    }

    // 通过相同的网卡接口直接发回，实现内核态极速反射转发！
    return XDP_TX;
}

char _license[] SEC("license") = "GPL";
