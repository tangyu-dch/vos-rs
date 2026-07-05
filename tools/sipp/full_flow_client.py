#!/usr/bin/env python3
"""
Complete SIP call flow client for VOS-RS testing.
Handles: REGISTER (digest auth) → INVITE → ACK → INFO (DTMF) → BYE
With RTP media sending during the call.
"""
import socket
import hashlib
import re
import time
import sys
import struct
import math
import random
import threading

SIP_PORT = 5160
SAMPLE_RATE = 8000
SAMPLES_PER_FRAME = 160
TONE_AMPLITUDE = 4000
PCMU_BIAS = 0x84
PCMU_CLIP = 32635


def encode_pcmu(sample):
    sign = 0
    if sample < 0:
        sign = 0x80
        sample = -sample
    sample = min(sample + PCMU_BIAS, PCMU_CLIP)
    exponent = 7
    mask = 0x4000
    while exponent > 0 and not (sample & mask):
        exponent -= 1
        mask >>= 1
    mantissa = (sample >> (exponent + 3)) & 0x0F
    return ~(sign | (exponent << 4) | mantissa) & 0xFF


def make_rtp_packet(seq, ts, ssrc, payload):
    header = struct.pack('!BBHII',
        (2 << 6) | 0, 0, seq & 0xffff, ts & 0xffffffff, ssrc & 0xffffffff)
    return header + payload


def generate_pcmu_frame(freq=440.0):
    payload = bytearray()
    phase_inc = 2.0 * math.pi * freq / SAMPLE_RATE
    phase = random.random() * 2 * math.pi
    for _ in range(SAMPLES_PER_FRAME):
        val = int(TONE_AMPLITUDE * math.sin(phase))
        val = max(-32768, min(32767, val))
        payload.append(encode_pcmu(val))
        phase += phase_inc
        if phase > 2 * math.pi:
            phase -= 2 * math.pi
    return bytes(payload)


class SipClient:
    def __init__(self, local_ip, local_port, remote_ip, remote_port):
        self.local_ip = local_ip
        self.local_port = local_port
        self.remote_ip = remote_ip
        self.remote_port = remote_port
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind((local_ip, local_port))
        self.sock.settimeout(5)
        self.call_id = f'call-{int(time.time())}-{random.randint(1000,9999)}@{local_ip}'
        self.from_tag = f'{int(time.time())}tag{random.randint(1000,9999)}'
        self.to_tag = None
        self.cseq = 0
        self.branch_counter = 0

    def next_branch(self):
        self.branch_counter += 1
        return f'z9hG4bK-{self.branch_counter}-{random.randint(1000,9999)}'

    def next_cseq(self):
        self.cseq += 1
        return self.cseq

    def send_sip(self, msg):
        self.sock.sendto(msg.encode(), (self.remote_ip, self.remote_port))

    def recv_sip(self, timeout=5):
        old_timeout = self.sock.gettimeout()
        self.sock.settimeout(timeout)
        try:
            data, addr = self.sock.recvfrom(65535)
            return data.decode(), addr
        except socket.timeout:
            return None, None
        finally:
            self.sock.settimeout(old_timeout)

    def parse_status(self, data):
        first_line = data.split('\r\n')[0]
        match = re.search(r'SIP/2\.0 (\d+)', first_line)
        return int(match.group(1)) if match else 0

    def parse_header(self, data, name):
        for line in data.split('\r\n'):
            if line.lower().startswith(name.lower() + ':'):
                return line.split(':', 1)[1].strip()
        return None

    def compute_digest(self, method, uri, username, password, realm, nonce, qop='auth', nc='00000001', cnonce='c1'):
        ha1 = hashlib.md5(f'{username}:{realm}:{password}'.encode()).hexdigest()
        ha2 = hashlib.md5(f'{method}:{uri}'.encode()).hexdigest()
        if qop:
            return hashlib.md5(f'{ha1}:{nonce}:{nc}:{cnonce}:{qop}:{ha2}'.encode()).hexdigest()
        else:
            return hashlib.md5(f'{ha1}:{nonce}:{ha2}'.encode()).hexdigest()

    def register(self, username, password):
        uri = f'sip:{username}@{self.remote_ip}:{self.remote_port}'
        cseq = self.next_cseq()

        reg = (f'REGISTER {uri} SIP/2.0\r\n'
               f'Via: SIP/2.0/UDP {self.local_ip}:{self.local_port};branch={self.next_branch()}\r\n'
               f'Max-Forwards: 70\r\n'
               f'From: "{username}" <sip:{username}@{self.local_ip}:{self.local_port}>;tag={self.from_tag}\r\n'
               f'To: "{username}" <sip:{username}@{self.local_ip}:{self.local_port}>\r\n'
               f'Call-ID: {self.call_id}\r\n'
               f'CSeq: {cseq} REGISTER\r\n'
               f'Contact: <sip:{username}@{self.local_ip}:{self.local_port}>\r\n'
               f'Expires: 3600\r\n'
               f'Content-Length: 0\r\n\r\n')
        self.send_sip(reg)
        data, _ = self.recv_sip()
        if not data:
            print('  ✗ No response to REGISTER')
            return False
        status = self.parse_status(data)
        print(f'  C→E  REGISTER (no auth)')
        print(f'  E→C  {data.split(chr(13)+chr(10))[0]}')

        if status == 401:
            www_auth = self.parse_header(data, 'WWW-Authenticate')
            realm = re.search(r'realm="([^"]+)"', www_auth).group(1)
            nonce = re.search(r'nonce="([^"]+)"', www_auth).group(1)
            qop_match = re.search(r'qop="([^"]*)"', www_auth)
            qop = qop_match.group(1) if qop_match else ''

            auth_resp = self.compute_digest('REGISTER', uri, username, password, realm, nonce, qop)

            cseq = self.next_cseq()
            reg_auth = (f'REGISTER {uri} SIP/2.0\r\n'
                       f'Via: SIP/2.0/UDP {self.local_ip}:{self.local_port};branch={self.next_branch()}\r\n'
                       f'Max-Forwards: 70\r\n'
                       f'From: "{username}" <sip:{username}@{self.local_ip}:{self.local_port}>;tag={self.from_tag}\r\n'
                       f'To: "{username}" <sip:{username}@{self.local_ip}:{self.local_port}>\r\n'
                       f'Call-ID: {self.call_id}\r\n'
                       f'CSeq: {cseq} REGISTER\r\n'
                       f'Contact: <sip:{username}@{self.local_ip}:{self.local_port}>\r\n'
                       f'Expires: 3600\r\n'
                       f'Authorization: Digest username="{username}", realm="{realm}", '
                       f'nonce="{nonce}", uri="{uri}", response="{auth_resp}", '
                       f'algorithm=MD5, qop={qop}, nc=00000001, cnonce="c1"\r\n'
                       f'Content-Length: 0\r\n\r\n')
            self.send_sip(reg_auth)
            data, _ = self.recv_sip()
            if not data:
                print('  ✗ No response to REGISTER+auth')
                return False
            status = self.parse_status(data)
            print(f'  C→E  REGISTER (digest auth)')
            print(f'  E→C  {data.split(chr(13)+chr(10))[0]}')

        return status == 200

    def _build_invite(self, remote_uri, username, extra_headers=''):
        cseq = self.next_cseq()
        local_tag = self.from_tag
        uri = remote_uri

        sdp = (f'v=0\r\n'
               f'o=caller 1 1 IN IP4 {self.local_ip}\r\n'
               f's=VOS-RS Python UAC\r\n'
               f'c=IN IP4 {self.local_ip}\r\n'
               f't=0 0\r\n'
               f'm=audio 20000 RTP/AVP 0 8 101\r\n'
               f'a=rtpmap:0 PCMU/8000\r\n'
               f'a=rtpmap:8 PCMA/8000\r\n'
               f'a=rtpmap:101 telephone-event/8000\r\n'
               f'a=fmtp:101 0-16\r\n')

        invite = (f'INVITE {uri} SIP/2.0\r\n'
                  f'Via: SIP/2.0/UDP {self.local_ip}:{self.local_port};branch={self.next_branch()}\r\n'
                  f'Max-Forwards: 70\r\n'
                  f'From: "{username}" <sip:{username}@{self.local_ip}:{self.local_port}>;tag={local_tag}\r\n'
                  f'To: <{uri}>\r\n'
                  f'Call-ID: {self.call_id}\r\n'
                  f'CSeq: {cseq} INVITE\r\n'
                  f'Contact: <sip:{username}@{self.local_ip}:{self.local_port}>\r\n'
                  f'{extra_headers}'
                  f'Content-Type: application/sdp\r\n'
                  f'Content-Length: {len(sdp)}\r\n\r\n{sdp}')
        return invite, uri

    def invite(self, remote_uri, username='1001', password='test1234'):
        uri = remote_uri
        invite_msg, _ = self._build_invite(remote_uri, username)
        self.send_sip(invite_msg)
        print(f'  C→E  INVITE (SDP: PCMU/PCMA/telephone-event)')
        auth_retries = 0

        while True:
            data, _ = self.recv_sip(timeout=10)
            if not data:
                print('  ✗ Timeout waiting for response')
                return False
            status = self.parse_status(data)
            first_line = data.split('\r\n')[0]
            print(f'  E→C  {first_line}')

            if status == 100:
                continue
            if status in (180, 183):
                continue
            if status == 407:
                auth_retries += 1
                if auth_retries > 3:
                    print(f'  ✗ Too many auth retries')
                    return False
                proxy_auth = self.parse_header(data, 'Proxy-Authenticate')
                realm = re.search(r'realm="([^"]+)"', proxy_auth).group(1)
                nonce = re.search(r'nonce="([^"]+)"', proxy_auth).group(1)
                qop_match = re.search(r'qop="([^"]*)"', proxy_auth)
                qop = qop_match.group(1) if qop_match else ''
                auth_resp = self.compute_digest('INVITE', uri, username, password, realm, nonce, qop)
                print(f'  → digest: realm={realm}, nonce={nonce[:20]}..., qop={qop}, response={auth_resp[:16]}...')

                extra = (f'Proxy-Authorization: Digest username="{username}", realm="{realm}", '
                         f'nonce="{nonce}", uri="{uri}", response="{auth_resp}", '
                         f'algorithm=MD5, qop={qop}, nc=00000001, cnonce="c1"\r\n')
                invite_auth, _ = self._build_invite(remote_uri, username, extra)
                self.send_sip(invite_auth)
                print(f'  C→E  INVITE (Proxy-Authorization)')
                continue
            if status == 200:
                to_header = self.parse_header(data, 'To')
                tag_match = re.search(r'tag=([^\s;]+)', to_header)
                if tag_match:
                    self.to_tag = tag_match.group(1)

                body_start = data.find('\r\n\r\n')
                if body_start >= 0:
                    body = data[body_start+4:]
                    media_match = re.search(r'm=audio (\d+)', body)
                    if media_match:
                        self.remote_media_port = int(media_match.group(1))
                        print(f'  → Remote media port: {self.remote_media_port}')

                self.send_ack()
                return True
            if status >= 400:
                print(f'  ✗ INVITE failed: {status}')
                return False

    def send_ack(self):
        cseq = self.next_cseq()
        to_hdr = f'<sip:13800138000@{self.remote_ip}:{self.remote_port}>'
        if self.to_tag:
            to_hdr += f';tag={self.to_tag}'

        ack = (f'ACK {to_hdr} SIP/2.0\r\n'
               f'Via: SIP/2.0/UDP {self.local_ip}:{self.local_port};branch={self.next_branch()}\r\n'
               f'Max-Forwards: 70\r\n'
               f'From: "1001" <sip:1001@{self.local_ip}:{self.local_port}>;tag={self.from_tag}\r\n'
               f'To: {to_hdr}\r\n'
               f'Call-ID: {self.call_id}\r\n'
               f'CSeq: {cseq} ACK\r\n'
               f'Content-Length: 0\r\n\r\n')
        self.send_sip(ack)
        print(f'  C→E  ACK')

    def _send_with_proxy_auth(self, method, uri, extra_headers, body='', content_type=''):
        cseq = self.next_cseq()
        to_hdr = f'<sip:13800138000@{self.remote_ip}:{self.remote_port}>'
        if self.to_tag:
            to_hdr += f';tag={self.to_tag}'

        msg = (f'{method} {to_hdr} SIP/2.0\r\n'
               f'Via: SIP/2.0/UDP {self.local_ip}:{self.local_port};branch={self.next_branch()}\r\n'
               f'Max-Forwards: 70\r\n'
               f'From: "1001" <sip:1001@{self.local_ip}:{self.local_port}>;tag={self.from_tag}\r\n'
               f'To: {to_hdr}\r\n'
               f'Call-ID: {self.call_id}\r\n'
               f'CSeq: {cseq} {method}\r\n'
               f'Contact: <sip:1001@{self.local_ip}:{self.local_port}>\r\n'
               f'{extra_headers}')
        if body:
            msg += f'Content-Type: {content_type}\r\nContent-Length: {len(body)}\r\n\r\n{body}'
        else:
            msg += 'Content-Length: 0\r\n\r\n'
        return msg

    def _handle_auth_challenge(self, data, method, uri, username, password, body='', content_type=''):
        auth_header = self.parse_header(data, 'Proxy-Authenticate')
        if not auth_header:
            return False
        realm = re.search(r'realm="([^"]+)"', auth_header).group(1)
        nonce = re.search(r'nonce="([^"]+)"', auth_header).group(1)
        qop_match = re.search(r'qop="([^"]*)"', auth_header)
        qop = qop_match.group(1) if qop_match else ''
        auth_resp = self.compute_digest(method, uri, username, password, realm, nonce, qop)
        extra = (f'Proxy-Authorization: Digest username="{username}", realm="{realm}", '
                 f'nonce="{nonce}", uri="{uri}", response="{auth_resp}", '
                 f'algorithm=MD5, qop={qop}, nc=00000001, cnonce="c1"\r\n')
        msg = self._send_with_proxy_auth(method, uri, extra, body, content_type)
        self.send_sip(msg)
        return True

    def send_dtmf(self, digit, duration=160, username='1001', password='test1234'):
        uri = f'sip:13800138000@{self.remote_ip}:{self.remote_port}'
        body = f'Signal={digit}\r\nDuration={duration}\r\n'
        msg = self._send_with_proxy_auth('INFO', uri, '', body, 'application/dtmf-relay')
        self.send_sip(msg)
        print(f'  C→E  INFO (DTMF: digit={digit})')

        while True:
            data, _ = self.recv_sip(timeout=5)
            if not data:
                print('  ✗ No response to INFO')
                return False
            status = self.parse_status(data)
            print(f'  E→C  {data.split(chr(13)+chr(10))[0]}')
            if status == 407:
                self._handle_auth_challenge(data, 'INFO', uri, username, password, body, 'application/dtmf-relay')
                print(f'  C→E  INFO (DTMF: digit={digit}) + Proxy-Auth')
                continue
            return status == 200

    def bye(self, username='1001', password='test1234'):
        uri = f'sip:13800138000@{self.remote_ip}:{self.remote_port}'
        msg = self._send_with_proxy_auth('BYE', uri, '')
        self.send_sip(msg)
        print(f'  C→E  BYE')

        while True:
            data, _ = self.recv_sip(timeout=5)
            if not data:
                print('  ✗ No response to BYE')
                return False
            status = self.parse_status(data)
            print(f'  E→C  {data.split(chr(13)+chr(10))[0]}')
            if status == 407:
                self._handle_auth_challenge(data, 'BYE', uri, username, password, '', '')
                print(f'  C→E  BYE + Proxy-Auth')
                continue
            return status == 200

    def close(self):
        self.sock.close()


def rtp_sender_thread(target_ip, target_port, duration, freq=440.0):
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    ssrc = random.randint(1000, 99999)
    interval = 1.0 / 50
    frame = generate_pcmu_frame(freq)
    total = 0
    start = time.time()
    while time.time() - start < duration:
        ts = (total * SAMPLES_PER_FRAME) & 0xffffffff
        pkt = make_rtp_packet(total & 0xffff, ts, ssrc, frame)
        sock.sendto(pkt, (target_ip, target_port))
        total += 1
        time.sleep(interval)
    sock.close()
    return total


def main():
    if len(sys.argv) < 3:
        print(f'Usage: {sys.argv[0]} <remote_ip> <remote_port> [username] [password]')
        sys.exit(1)

    remote_ip = sys.argv[1]
    remote_port = int(sys.argv[2])
    username = sys.argv[3] if len(sys.argv) > 3 else '1001'
    password = sys.argv[4] if len(sys.argv) > 4 else 'test1234'

    print(f'SIP Call Flow Client: {username}@{remote_ip}:{remote_port}')
    print('=' * 60)

    client = SipClient('127.0.0.1', 5164, remote_ip, remote_port)
    uri = f'sip:13800138000@{remote_ip}:{remote_port}'

    # Phase 1: REGISTER
    print(f'\n── Phase 1: REGISTER (Digest Auth) ──')
    if not client.register(username, password):
        print('REGISTER FAILED')
        client.close()
        sys.exit(1)
    print(f'  ✓ Registration successful')
    time.sleep(0.5)

    # Phase 2: INVITE
    print(f'\n── Phase 2: INVITE → Media → DTMF → BYE ──')
    if not client.invite(uri, username):
        print('INVITE FAILED')
        client.close()
        sys.exit(1)

    # Start RTP senders
    gw_port = getattr(client, 'remote_media_port', 40000)
    caller_port = 20000
    print(f'\n── Phase 3: RTP Media ({gw_port}/{caller_port}) ──')

    t1 = threading.Thread(target=rtp_sender_thread, args=(remote_ip, gw_port, 8, 440.0))
    t2 = threading.Thread(target=rtp_sender_thread, args=(remote_ip, caller_port, 8, 880.0))
    t1.start()
    t2.start()

    print(f'  → RTP media flowing (440Hz + 880Hz)')

    # Phase 4: Hold for 3 seconds
    print(f'\n── Phase 4: Call Active (3s) ──')
    time.sleep(3)

    # Phase 5: DTMF
    print(f'\n── Phase 5: DTMF ──')
    client.send_dtmf(5, 160)
    time.sleep(0.5)

    # Phase 6: BYE
    print(f'\n── Phase 6: BYE ──')
    client.bye()

    # Wait for RTP to finish
    t1.join(timeout=5)
    t2.join(timeout=5)

    client.close()
    print(f'\n═══════════════════════════════════════')
    print(f'  CALL COMPLETE')
    print(f'═══════════════════════════════════════')


if __name__ == '__main__':
    main()
