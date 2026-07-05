#!/usr/bin/env python3
import getpass
import hashlib
import os
import queue
import random
import re
import signal
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path


ROOT_DIR = Path(__file__).resolve().parents[2]
LOG_DIR = Path(os.environ.get("LOG_DIR", ROOT_DIR / "target" / "full-flow"))
LOCAL_IP = os.environ.get("LOCAL_IP", "127.0.0.1")
DESTINATION = os.environ.get("DESTINATION", "13800138000")
TIMEOUT_SECONDS = float(os.environ.get("FULL_FLOW_TIMEOUT", "8"))
GATEWAY_TAG = "gw-full-flow"
AUTH_REALM = "vos-rs"
AUTH_NONCE = "full-flow-nonce"
AUTH_PASSWORDS = {
    "1001": "secret",
    "1002": "secret",
}


class FullFlowError(Exception):
    pass


def main() -> int:
    LOG_DIR.mkdir(parents=True, exist_ok=True)
    for path in LOG_DIR.glob("*"):
        if path.is_file():
            path.unlink()

    call_id = f"full-flow-{int(time.time())}-{os.getpid()}@vos-rs.local"
    target_db_url = os.environ.get("VOS_RS_FULL_FLOW_DATABASE_URL")
    db_name = ""
    edge_process = None
    edge_log = None
    sockets = []

    try:
        if target_db_url:
            database_url = target_db_url
            print(f"Using target database: {database_url.split('@')[-1]}")
        else:
            require_postgres()
            db_name = f"vos_rs_full_flow_{os.getpid()}_{int(time.time())}"
            create_database(db_name)
            database_url = f"postgres://{getpass.getuser()}@localhost/{db_name}"

        print("Building sip-edge...")
        subprocess.run(["cargo", "build", "-p", "sip-edge"], cwd=ROOT_DIR, check=True)

        caller_sip = bind_udp(LOCAL_IP, 0)
        callee_sip = bind_udp(LOCAL_IP, 0)
        gateway_fixed_port = int(os.environ.get("VOS_RS_FULL_FLOW_GATEWAY_PORT", "0"))
        gateway_sip = bind_udp(LOCAL_IP, gateway_fixed_port)
        caller_media, caller_rtcp = bind_rtp_rtcp_pair(LOCAL_IP)
        callee_media, callee_rtcp = bind_rtp_rtcp_pair(LOCAL_IP)
        gateway_media, gateway_rtcp = bind_rtp_rtcp_pair(LOCAL_IP)
        sockets.extend(
            [
                caller_sip,
                callee_sip,
                gateway_sip,
                caller_media,
                caller_rtcp,
                callee_media,
                callee_rtcp,
                gateway_media,
                gateway_rtcp,
            ]
        )

        edge_port = unused_udp_port()
        rtp_min, rtp_max = unused_even_udp_pair()

        caller_port = caller_sip.getsockname()[1]
        callee_port = callee_sip.getsockname()[1]
        gateway_port = gateway_sip.getsockname()[1]
        caller_media_port = caller_media.getsockname()[1]
        callee_media_port = callee_media.getsockname()[1]
        gateway_media_port = gateway_media.getsockname()[1]

        edge_log = (LOG_DIR / "sip-edge.log").open("wb")
        edge_env = os.environ.copy()
        edge_env.pop("VOS_RS_NATS_URL", None)
        edge_env.update(
            {
                "VOS_RS_SIP_UDP_BIND": f"{LOCAL_IP}:{edge_port}",
                "VOS_RS_SIP_DEFAULT_GATEWAY": f"{LOCAL_IP}:{gateway_port}",
                "VOS_RS_SIP_ADVERTISED_ADDR": f"{LOCAL_IP}:{edge_port}",
                "VOS_RS_RTP_ADVERTISED_ADDR": LOCAL_IP,
                "VOS_RS_RTP_PORT_MIN": str(rtp_min),
                "VOS_RS_RTP_PORT_MAX": str(rtp_max),
                "VOS_RS_DATABASE_URL": database_url,
                "VOS_RS_SIP_AUTH_USERS": ",".join(
                    f"{user}:{password}" for user, password in AUTH_PASSWORDS.items()
                ),
                "VOS_RS_SIP_AUTH_REALM": AUTH_REALM,
                "VOS_RS_SIP_AUTH_NONCE": AUTH_NONCE,
                "VOS_RS_RECORDING_ENABLED": os.environ.get("VOS_RS_RECORDING_ENABLED", "true"),
                "VOS_RS_RECORDING_DIR": os.environ.get(
                    "VOS_RS_RECORDING_DIR", str(ROOT_DIR / "target" / "recordings")
                ),
                "RUST_LOG": edge_env.get("RUST_LOG", "sip_edge=debug"),
            }
        )
        edge_process = subprocess.Popen(
            [str(ROOT_DIR / "target" / "debug" / "sip-edge")],
            cwd=ROOT_DIR,
            env=edge_env,
            stdout=edge_log,
            stderr=subprocess.STDOUT,
        )
        wait_for_edge(edge_process, edge_port)

        gateway_result_queue: queue.Queue[dict] = queue.Queue()
        gateway_error_queue: queue.Queue[BaseException] = queue.Queue()
        gateway_thread = threading.Thread(
            target=gateway_uas,
            kwargs={
                "gateway_sip": gateway_sip,
                "gateway_media": gateway_media,
                "gateway_rtcp": gateway_rtcp,
                "gateway_media_port": gateway_media_port,
                "gateway_result_queue": gateway_result_queue,
                "gateway_error_queue": gateway_error_queue,
            },
            daemon=True,
        )
        gateway_thread.start()

        caller_result = caller_uac(
            caller_sip=caller_sip,
            caller_media=caller_media,
            caller_rtcp=caller_rtcp,
            caller_port=caller_port,
            caller_media_port=caller_media_port,
            edge_port=edge_port,
            call_id=call_id,
        )

        gateway_thread.join(TIMEOUT_SECONDS)
        if gateway_thread.is_alive():
            raise FullFlowError("gateway UAS thread did not finish")
        if not gateway_error_queue.empty():
            raise gateway_error_queue.get()
        gateway_result = gateway_result_queue.get_nowait()

        internal_call_id = f"internal-{call_id}"
        register_endpoint(
            udp_socket=callee_sip,
            user="1002",
            sip_port=callee_port,
            edge_port=edge_port,
            log_prefix="callee",
        )

        callee_result_queue: queue.Queue[dict] = queue.Queue()
        callee_error_queue: queue.Queue[BaseException] = queue.Queue()
        callee_thread = threading.Thread(
            target=registered_callee_uas,
            kwargs={
                "callee_sip": callee_sip,
                "callee_media": callee_media,
                "callee_media_port": callee_media_port,
                "callee_result_queue": callee_result_queue,
                "callee_error_queue": callee_error_queue,
            },
            daemon=True,
        )
        callee_thread.start()

        internal_result = internal_caller_uac(
            caller_sip=caller_sip,
            caller_media=caller_media,
            caller_port=caller_port,
            caller_media_port=caller_media_port,
            edge_port=edge_port,
            call_id=internal_call_id,
        )

        callee_thread.join(TIMEOUT_SECONDS)
        if callee_thread.is_alive():
            raise FullFlowError("registered callee UAS thread did not finish")
        if not callee_error_queue.empty():
            raise callee_error_queue.get()
        callee_result = callee_result_queue.get_nowait()

        cdr = wait_for_cdr(database_url, call_id)
        internal_cdr = wait_for_cdr(database_url, internal_call_id)

        # Verify DTMF digits in the CDR (allow any order of 5, 7, 9, 8 to avoid timing variations)
        digits = cdr.split(",")[-1]
        if sorted(digits) != sorted("5798"):
            raise FullFlowError(f"Expected CDR to contain DTMF digits '5798' in some order, but got: {cdr}")
        if not internal_cdr.endswith(","):
            raise FullFlowError(f"Expected internal CDR to end with ',' (empty DTMF), but got: {internal_cdr}")

        (LOG_DIR / "cdr.txt").write_text(cdr + "\n" + internal_cdr + "\n", encoding="utf-8")

        print("Full flow passed.")
        print(f"Call-ID: {call_id}")
        print(f"SIP edge: {LOCAL_IP}:{edge_port}")
        print(
            "RTP caller->gateway: "
            f"{LOCAL_IP}:{caller_result['answer_relay_port']} -> "
            f"{LOCAL_IP}:{gateway_media_port}"
        )
        print(
            "RTP gateway->caller: "
            f"{LOCAL_IP}:{gateway_result['offer_relay_port']} -> "
            f"{LOCAL_IP}:{caller_media_port}"
        )
        print(
            "RTCP caller<->gateway: "
            f"{LOCAL_IP}:{caller_result['answer_relay_port'] + 1} <-> "
            f"{LOCAL_IP}:{gateway_result['offer_relay_port'] + 1}"
        )
        print(
            "Registered route: "
            f"1002 -> {LOCAL_IP}:{callee_port}, "
            f"relay {LOCAL_IP}:{internal_result['answer_relay_port']}"
        )
        print(
            "RTP caller->registered callee: "
            f"{LOCAL_IP}:{internal_result['answer_relay_port']} -> "
            f"{LOCAL_IP}:{callee_media_port}"
        )
        print(
            "RTP registered callee->caller: "
            f"{LOCAL_IP}:{callee_result['offer_relay_port']} -> "
            f"{LOCAL_IP}:{caller_media_port}"
        )
        print(f"CDR: {cdr}")
        print(f"Internal CDR: {internal_cdr}")
        print(f"Logs: {LOG_DIR}")
        return 0
    except BaseException as error:
        print(f"Full flow failed: {error}", file=sys.stderr)
        print_failure_logs()
        return 1
    finally:
        for udp_socket in sockets:
            udp_socket.close()
        if edge_process is not None:
            terminate_process(edge_process)
        if edge_log is not None:
            edge_log.close()
        if db_name:
            drop_database(db_name)


def caller_uac(
    caller_sip: socket.socket,
    caller_media: socket.socket,
    caller_rtcp: socket.socket,
    caller_port: int,
    caller_media_port: int,
    edge_port: int,
    call_id: str,
) -> dict:
    edge_addr = (LOCAL_IP, edge_port)
    caller_sip.settimeout(TIMEOUT_SECONDS)
    caller_media.settimeout(TIMEOUT_SECONDS)
    caller_rtcp.settimeout(TIMEOUT_SECONDS)

    register_endpoint(caller_sip, "1001", caller_port, edge_port, "caller")

    invite = build_invite(call_id, caller_port, caller_media_port, edge_port)
    write_log("caller_invite.txt", invite)
    caller_sip.sendto(invite.encode("utf-8"), edge_addr)

    response_100 = recv_sip_text(caller_sip, "100 Trying")
    response_180 = recv_sip_text(caller_sip, "180 Ringing")
    response_200 = recv_sip_text(caller_sip, "200 OK")
    write_log("caller_response_100.txt", response_100)
    write_log("caller_response_180.txt", response_180)
    write_log("caller_response_200.txt", response_200)

    require_status(response_100, 100)
    require_status(response_180, 180)
    require_status(response_200, 200)
    answer_addr, answer_relay_port = parse_sdp_endpoint(response_200)

    ack = build_in_dialog_request("ACK", 1, call_id, caller_port, edge_port, response_200)
    write_log("caller_ack.txt", ack)
    caller_sip.sendto(ack.encode("utf-8"), edge_addr)

    caller_to_gateway_packet = rtp_packet(
        payload_type=8, sequence=1, timestamp=160, ssrc=0xC011EA, payload=b"caller"
    )
    caller_media.sendto(caller_to_gateway_packet, (answer_addr, answer_relay_port))

    # Send STUN Binding Request packet (first byte 0x01)
    stun_packet = b"\x01\x00\x00\x08\x21\x12\xa4\x42"
    caller_media.sendto(stun_packet, (answer_addr, answer_relay_port))

    # Send DTLS Client Hello packet (first byte 0x16)
    dtls_packet = b"\x16\x03\x01\x00\x50"
    caller_media.sendto(dtls_packet, (answer_addr, answer_relay_port))

    # Send DTMF packets (digit 5 and digit 9)
    dtmf_5_packet = rtp_packet(
        payload_type=101, sequence=2, timestamp=1000, ssrc=0xC011EA, payload=bytes([5, 0, 0, 80])
    )
    caller_media.sendto(dtmf_5_packet, (answer_addr, answer_relay_port))

    dtmf_9_packet = rtp_packet(
        payload_type=101, sequence=3, timestamp=2000, ssrc=0xC011EA, payload=bytes([9, 0, 0, 80])
    )
    caller_media.sendto(dtmf_9_packet, (answer_addr, answer_relay_port))

    # Send SIP INFO with application/dtmf-relay body (digit '7')
    info_relay = build_info_request(
        call_id, caller_port, edge_port, response_200, "application/dtmf-relay", "Signal= 7\r\nDuration= 160\r\n", 4
    )
    caller_sip.sendto(info_relay.encode("utf-8"), edge_addr)
    info_relay_resp = recv_sip_text(caller_sip, "200 OK for INFO relay")
    write_log("caller_info_relay_resp.txt", info_relay_resp)
    require_status(info_relay_resp, 200)

    # Send SIP INFO with application/dtmf body (digit '8')
    info_dtmf = build_info_request(
        call_id, caller_port, edge_port, response_200, "application/dtmf", "8", 5
    )
    caller_sip.sendto(info_dtmf.encode("utf-8"), edge_addr)
    info_dtmf_resp = recv_sip_text(caller_sip, "200 OK for INFO dtmf")
    write_log("caller_info_dtmf_resp.txt", info_dtmf_resp)
    require_status(info_dtmf_resp, 200)
    caller_to_gateway_rtcp = rtcp_receiver_report(
        ssrc=0xC011EA,
        source_ssrc=0x6A7EAA,
        fraction_lost=1,
        cumulative_lost=2,
        jitter=12,
        last_sender_report=0x01020304,
        delay_since_last_sender_report=5,
    )
    caller_rtcp.sendto(caller_to_gateway_rtcp, (answer_addr, answer_relay_port + 1))

    gateway_to_caller_packet, _ = caller_media.recvfrom(1500)
    expected_gateway_packet = rtp_packet(
        payload_type=8, sequence=2, timestamp=320, ssrc=0x6A7EAA, payload=b"gateway"
    )
    if gateway_to_caller_packet != expected_gateway_packet:
        raise FullFlowError("caller did not receive expected gateway RTP packet")

    gateway_to_caller_rtcp, _ = caller_rtcp.recvfrom(1500)
    expected_gateway_rtcp = rtcp_receiver_report(
        ssrc=0x6A7EAA,
        source_ssrc=0xC011EA,
        fraction_lost=2,
        cumulative_lost=3,
        jitter=14,
        last_sender_report=0x05060708,
        delay_since_last_sender_report=6,
    )
    if gateway_to_caller_rtcp != expected_gateway_rtcp:
        raise FullFlowError("caller did not receive expected gateway RTCP packet")

    bye = build_in_dialog_request("BYE", 6, call_id, caller_port, edge_port, response_200)
    write_log("caller_bye.txt", bye)
    caller_sip.sendto(bye.encode("utf-8"), edge_addr)
    bye_ok = recv_sip_text(caller_sip, "200 OK for BYE")
    write_log("caller_bye_ok.txt", bye_ok)
    require_status(bye_ok, 200)

    return {"answer_relay_port": answer_relay_port}


def internal_caller_uac(
    caller_sip: socket.socket,
    caller_media: socket.socket,
    caller_port: int,
    caller_media_port: int,
    edge_port: int,
    call_id: str,
) -> dict:
    edge_addr = (LOCAL_IP, edge_port)
    caller_sip.settimeout(TIMEOUT_SECONDS)
    caller_media.settimeout(TIMEOUT_SECONDS)

    invite = build_invite(call_id, caller_port, caller_media_port, edge_port, destination="1002")
    write_log("internal_caller_invite.txt", invite)
    caller_sip.sendto(invite.encode("utf-8"), edge_addr)

    response_100 = recv_sip_text(caller_sip, "100 Trying for internal call")
    response_180 = recv_sip_text(caller_sip, "180 Ringing for internal call")
    response_200 = recv_sip_text(caller_sip, "200 OK for internal call")
    write_log("internal_caller_response_100.txt", response_100)
    write_log("internal_caller_response_180.txt", response_180)
    write_log("internal_caller_response_200.txt", response_200)

    require_status(response_100, 100)
    require_status(response_180, 180)
    require_status(response_200, 200)
    answer_addr, answer_relay_port = parse_sdp_endpoint(response_200)

    ack = build_in_dialog_request(
        "ACK",
        1,
        call_id,
        caller_port,
        edge_port,
        response_200,
        destination="1002",
    )
    write_log("internal_caller_ack.txt", ack)
    caller_sip.sendto(ack.encode("utf-8"), edge_addr)

    caller_to_callee_packet = rtp_packet(
        payload_type=0, sequence=3, timestamp=480, ssrc=0x10020001, payload=b"caller2"
    )
    caller_media.sendto(caller_to_callee_packet, (answer_addr, answer_relay_port))

    callee_to_caller_packet, _ = caller_media.recvfrom(1500)
    expected_callee_packet = rtp_packet(
        payload_type=0, sequence=4, timestamp=640, ssrc=0x10020002, payload=b"callee2"
    )
    if callee_to_caller_packet != expected_callee_packet:
        raise FullFlowError("caller did not receive expected registered callee RTP packet")

    bye = build_in_dialog_request(
        "BYE",
        2,
        call_id,
        caller_port,
        edge_port,
        response_200,
        destination="1002",
    )
    write_log("internal_caller_bye.txt", bye)
    caller_sip.sendto(bye.encode("utf-8"), edge_addr)
    bye_ok = recv_sip_text(caller_sip, "200 OK for internal BYE")
    write_log("internal_caller_bye_ok.txt", bye_ok)
    require_status(bye_ok, 200)

    return {"answer_relay_port": answer_relay_port}


def gateway_uas(
    gateway_sip: socket.socket,
    gateway_media: socket.socket,
    gateway_rtcp: socket.socket,
    gateway_media_port: int,
    gateway_result_queue: queue.Queue,
    gateway_error_queue: queue.Queue,
) -> None:
    try:
        gateway_sip.settimeout(TIMEOUT_SECONDS)
        gateway_media.settimeout(TIMEOUT_SECONDS)
        gateway_rtcp.settimeout(TIMEOUT_SECONDS)

        invite_bytes, edge_addr = gateway_sip.recvfrom(65535)
        invite = invite_bytes.decode("utf-8", errors="replace")
        write_log("gateway_invite.txt", invite)
        offer_addr, offer_relay_port = parse_sdp_endpoint(invite)
        require_audio_payloads(invite, {"0", "8"})

        ringing = build_gateway_response(180, "Ringing", invite)
        write_log("gateway_180.txt", ringing)
        gateway_sip.sendto(ringing.encode("utf-8"), edge_addr)

        ok = build_gateway_response(
            200,
            "OK",
            invite,
            body=sdp_body("gateway", gateway_media_port, payload_types=(8, 101)),
            content_type="application/sdp",
        )
        write_log("gateway_200.txt", ok)
        gateway_sip.sendto(ok.encode("utf-8"), edge_addr)

        ack_bytes, _ = gateway_sip.recvfrom(65535)
        ack = ack_bytes.decode("utf-8", errors="replace")
        write_log("gateway_ack.txt", ack)
        if not ack.startswith("ACK "):
            raise FullFlowError("gateway expected ACK")

        caller_to_gateway_packet, _ = gateway_media.recvfrom(1500)
        expected_caller_packet = rtp_packet(
            payload_type=8, sequence=1, timestamp=160, ssrc=0xC011EA, payload=b"caller"
        )
        if caller_to_gateway_packet != expected_caller_packet:
            raise FullFlowError("gateway did not receive expected caller RTP packet")

        # Receive and verify STUN pass-through packet
        caller_to_gateway_stun, _ = gateway_media.recvfrom(1500)
        if caller_to_gateway_stun != b"\x01\x00\x00\x08\x21\x12\xa4\x42":
            raise FullFlowError("gateway did not receive expected caller STUN pass-through packet")

        # Receive and verify DTLS pass-through packet
        caller_to_gateway_dtls, _ = gateway_media.recvfrom(1500)
        if caller_to_gateway_dtls != b"\x16\x03\x01\x00\x50":
            raise FullFlowError("gateway did not receive expected caller DTLS pass-through packet")

        # Receive forwarded INFO (relay)
        info_relay_bytes, _ = gateway_sip.recvfrom(65535)
        info_relay_str = info_relay_bytes.decode("utf-8", errors="replace")
        if "INFO " not in info_relay_str:
            raise FullFlowError("gateway expected forwarded INFO relay request")
        info_relay_ok = build_gateway_response(200, "OK", info_relay_str, add_to_tag=False)
        gateway_sip.sendto(info_relay_ok.encode("utf-8"), edge_addr)

        # Receive forwarded INFO (dtmf)
        info_dtmf_bytes, _ = gateway_sip.recvfrom(65535)
        info_dtmf_str = info_dtmf_bytes.decode("utf-8", errors="replace")
        if "INFO " not in info_dtmf_str:
            raise FullFlowError("gateway expected forwarded INFO dtmf request")
        info_dtmf_ok = build_gateway_response(200, "OK", info_dtmf_str, add_to_tag=False)
        gateway_sip.sendto(info_dtmf_ok.encode("utf-8"), edge_addr)

        caller_to_gateway_rtcp, _ = gateway_rtcp.recvfrom(1500)
        expected_caller_rtcp = rtcp_receiver_report(
            ssrc=0xC011EA,
            source_ssrc=0x6A7EAA,
            fraction_lost=1,
            cumulative_lost=2,
            jitter=12,
            last_sender_report=0x01020304,
            delay_since_last_sender_report=5,
        )
        if caller_to_gateway_rtcp != expected_caller_rtcp:
            raise FullFlowError("gateway did not receive expected caller RTCP packet")

        gateway_to_caller_packet = rtp_packet(
            payload_type=8, sequence=2, timestamp=320, ssrc=0x6A7EAA, payload=b"gateway"
        )
        gateway_media.sendto(gateway_to_caller_packet, (offer_addr, offer_relay_port))
        gateway_to_caller_rtcp = rtcp_receiver_report(
            ssrc=0x6A7EAA,
            source_ssrc=0xC011EA,
            fraction_lost=2,
            cumulative_lost=3,
            jitter=14,
            last_sender_report=0x05060708,
            delay_since_last_sender_report=6,
        )
        gateway_rtcp.sendto(gateway_to_caller_rtcp, (offer_addr, offer_relay_port + 1))

        bye_bytes, edge_addr = gateway_sip.recvfrom(65535)
        bye = bye_bytes.decode("utf-8", errors="replace")
        write_log("gateway_bye.txt", bye)
        if not bye.startswith("BYE "):
            raise FullFlowError("gateway expected BYE")

        bye_ok = build_gateway_response(200, "OK", bye, add_to_tag=False)
        write_log("gateway_bye_ok.txt", bye_ok)
        gateway_sip.sendto(bye_ok.encode("utf-8"), edge_addr)

        gateway_result_queue.put({"offer_relay_port": offer_relay_port})
    except BaseException as error:
        gateway_error_queue.put(error)


def registered_callee_uas(
    callee_sip: socket.socket,
    callee_media: socket.socket,
    callee_media_port: int,
    callee_result_queue: queue.Queue,
    callee_error_queue: queue.Queue,
) -> None:
    try:
        callee_sip.settimeout(TIMEOUT_SECONDS)
        callee_media.settimeout(TIMEOUT_SECONDS)

        invite_bytes, edge_addr = callee_sip.recvfrom(65535)
        invite = invite_bytes.decode("utf-8", errors="replace")
        write_log("callee_invite.txt", invite)
        offer_addr, offer_relay_port = parse_sdp_endpoint(invite)
        require_audio_payloads(invite, {"0", "8"})

        ringing = build_gateway_response(180, "Ringing", invite)
        write_log("callee_180.txt", ringing)
        callee_sip.sendto(ringing.encode("utf-8"), edge_addr)

        ok = build_gateway_response(
            200,
            "OK",
            invite,
            body=sdp_body("callee", callee_media_port, payload_types=(0,)),
            content_type="application/sdp",
        )
        write_log("callee_200.txt", ok)
        callee_sip.sendto(ok.encode("utf-8"), edge_addr)

        ack_bytes, _ = callee_sip.recvfrom(65535)
        ack = ack_bytes.decode("utf-8", errors="replace")
        write_log("callee_ack.txt", ack)
        if not ack.startswith("ACK "):
            raise FullFlowError("registered callee expected ACK")

        caller_to_callee_packet, _ = callee_media.recvfrom(1500)
        expected_caller_packet = rtp_packet(
            payload_type=0, sequence=3, timestamp=480, ssrc=0x10020001, payload=b"caller2"
        )
        if caller_to_callee_packet != expected_caller_packet:
            raise FullFlowError("registered callee did not receive expected caller RTP packet")

        callee_to_caller_packet = rtp_packet(
            payload_type=0, sequence=4, timestamp=640, ssrc=0x10020002, payload=b"callee2"
        )
        callee_media.sendto(callee_to_caller_packet, (offer_addr, offer_relay_port))

        bye_bytes, edge_addr = callee_sip.recvfrom(65535)
        bye = bye_bytes.decode("utf-8", errors="replace")
        write_log("callee_bye.txt", bye)
        if not bye.startswith("BYE "):
            raise FullFlowError("registered callee expected BYE")

        bye_ok = build_gateway_response(200, "OK", bye, add_to_tag=False)
        write_log("callee_bye_ok.txt", bye_ok)
        callee_sip.sendto(bye_ok.encode("utf-8"), edge_addr)

        callee_result_queue.put({"offer_relay_port": offer_relay_port})
    except BaseException as error:
        callee_error_queue.put(error)


def register_endpoint(
    udp_socket: socket.socket,
    user: str,
    sip_port: int,
    edge_port: int,
    log_prefix: str,
) -> None:
    edge_addr = (LOCAL_IP, edge_port)
    udp_socket.settimeout(TIMEOUT_SECONDS)

    register = build_register(user, sip_port, edge_port, authorization=None, cseq=1)
    write_log(f"{log_prefix}_register.txt", register)
    udp_socket.sendto(register.encode("utf-8"), edge_addr)

    challenge = recv_sip_text(udp_socket, f"401 challenge for {log_prefix} REGISTER")
    write_log(f"{log_prefix}_register_challenge.txt", challenge)
    require_status(challenge, 401)

    request_uri = f"sip:{LOCAL_IP}:{edge_port}"
    authorization = build_digest_authorization(
        user=user,
        password=AUTH_PASSWORDS[user],
        method="REGISTER",
        uri=request_uri,
        challenge=challenge,
    )
    register = build_register(user, sip_port, edge_port, authorization=authorization, cseq=2)
    write_log(f"{log_prefix}_register_authorized.txt", register)
    udp_socket.sendto(register.encode("utf-8"), edge_addr)

    register_ok = recv_sip_text(udp_socket, f"200 OK for authorized {log_prefix} REGISTER")
    write_log(f"{log_prefix}_register_ok.txt", register_ok)
    require_status(register_ok, 200)
    if f"Contact: <sip:{user}@" not in register_ok:
        raise FullFlowError(f"{log_prefix} REGISTER response did not include registered Contact")


def build_register(
    user: str,
    sip_port: int,
    edge_port: int,
    authorization: str | None,
    cseq: int,
) -> str:
    headers = [
        ("Via", f"SIP/2.0/UDP {LOCAL_IP}:{sip_port};branch=z9hG4bK-full-flow-register-{user}-{cseq}"),
        ("Max-Forwards", "70"),
        ("From", f'"{user}" <sip:{user}@{LOCAL_IP}:{sip_port}>;tag={user}-full-flow'),
        ("To", f"<sip:{user}@{LOCAL_IP}:{edge_port}>"),
        ("Call-ID", f"register-{user}-{os.getpid()}@vos-rs.local"),
        ("CSeq", f"{cseq} REGISTER"),
        ("Contact", f"<sip:{user}@{LOCAL_IP}:{sip_port};transport=udp>;expires=120"),
    ]
    if authorization is not None:
        headers.insert(-1, ("Authorization", authorization))

    return sip_request(
        method="REGISTER",
        request_uri=f"sip:{LOCAL_IP}:{edge_port}",
        headers=headers,
    )


def build_digest_authorization(
    user: str,
    password: str,
    method: str,
    uri: str,
    challenge: str,
) -> str:
    www_authenticate = header_value(challenge, "WWW-Authenticate")
    params = parse_digest_params(www_authenticate)
    realm = params.get("realm", AUTH_REALM)
    nonce = params.get("nonce", AUTH_NONCE)
    qop = "auth"
    nc = "00000001"
    cnonce = f"{os.getpid():x}{int(time.time()):x}"
    response = digest_response(
        username=user,
        password=password,
        realm=realm,
        nonce=nonce,
        method=method,
        uri=uri,
        qop=qop,
        nc=nc,
        cnonce=cnonce,
    )

    return (
        f'Digest username="{user}", realm="{realm}", nonce="{nonce}", '
        f'uri="{uri}", response="{response}", algorithm=MD5, '
        f"qop={qop}, nc={nc}, cnonce=\"{cnonce}\""
    )


def digest_response(
    username: str,
    password: str,
    realm: str,
    nonce: str,
    method: str,
    uri: str,
    qop: str,
    nc: str,
    cnonce: str,
) -> str:
    ha1 = md5_hex(f"{username}:{realm}:{password}")
    ha2 = md5_hex(f"{method}:{uri}")
    return md5_hex(f"{ha1}:{nonce}:{nc}:{cnonce}:{qop}:{ha2}")


def md5_hex(value: str) -> str:
    return hashlib.md5(value.encode("utf-8")).hexdigest()


def parse_digest_params(header: str) -> dict[str, str]:
    header = header.strip()
    if header.lower().startswith("digest "):
        header = header[7:]

    params: dict[str, str] = {}
    index = 0
    while index < len(header):
        while index < len(header) and header[index] in " ,":
            index += 1
        key_start = index
        while index < len(header) and header[index] != "=":
            index += 1
        if index >= len(header):
            break
        key = header[key_start:index].strip().lower()
        index += 1
        if index < len(header) and header[index] == '"':
            index += 1
            value = []
            escaped = False
            while index < len(header):
                char = header[index]
                index += 1
                if escaped:
                    value.append(char)
                    escaped = False
                elif char == "\\":
                    escaped = True
                elif char == '"':
                    break
                else:
                    value.append(char)
            params[key] = "".join(value)
        else:
            value_start = index
            while index < len(header) and header[index] != ",":
                index += 1
            params[key] = header[value_start:index].strip()

    return params


def build_invite(
    call_id: str,
    caller_port: int,
    caller_media_port: int,
    edge_port: int,
    destination: str = DESTINATION,
) -> str:
    body = sdp_body("caller", caller_media_port)
    return sip_request(
        method="INVITE",
        request_uri=f"sip:{destination}@{LOCAL_IP}:{edge_port}",
        headers=[
            ("Via", f"SIP/2.0/UDP {LOCAL_IP}:{caller_port};branch=z9hG4bK-full-flow-invite"),
            ("Max-Forwards", "70"),
            ("From", f'"1001" <sip:1001@{LOCAL_IP}:{caller_port}>;tag=caller-full-flow'),
            ("To", f"<sip:{destination}@{LOCAL_IP}:{edge_port}>"),
            ("Call-ID", call_id),
            ("CSeq", "1 INVITE"),
            ("Contact", f"<sip:1001@{LOCAL_IP}:{caller_port}>"),
            ("Content-Type", "application/sdp"),
        ],
        body=body,
    )


def build_in_dialog_request(
    method: str,
    cseq: int,
    call_id: str,
    caller_port: int,
    edge_port: int,
    ok_response: str,
    destination: str = DESTINATION,
) -> str:
    to_header = header_value(ok_response, "To")
    return sip_request(
        method=method,
        request_uri=f"sip:{destination}@{LOCAL_IP}:{edge_port}",
        headers=[
            ("Via", f"SIP/2.0/UDP {LOCAL_IP}:{caller_port};branch=z9hG4bK-full-flow-{method.lower()}"),
            ("Max-Forwards", "70"),
            ("From", f'"1001" <sip:1001@{LOCAL_IP}:{caller_port}>;tag=caller-full-flow'),
            ("To", to_header),
            ("Call-ID", call_id),
            ("CSeq", f"{cseq} {method}"),
            ("Contact", f"<sip:1001@{LOCAL_IP}:{caller_port}>"),
        ],
    )


def build_info_request(
    call_id: str,
    caller_port: int,
    edge_port: int,
    ok_response: str,
    content_type: str,
    body: str,
    cseq: int,
) -> str:
    to_header = header_value(ok_response, "To")
    return sip_request(
        method="INFO",
        request_uri=f"sip:{DESTINATION}@{LOCAL_IP}:{edge_port}",
        headers=[
            ("Via", f"SIP/2.0/UDP {LOCAL_IP}:{caller_port};branch=z9hG4bK-full-flow-info-{cseq}"),
            ("Max-Forwards", "70"),
            ("From", f'"1001" <sip:1001@{LOCAL_IP}:{caller_port}>;tag=caller-full-flow'),
            ("To", to_header),
            ("Call-ID", call_id),
            ("CSeq", f"{cseq} INFO"),
            ("Contact", f"<sip:1001@{LOCAL_IP}:{caller_port}>"),
            ("Content-Type", content_type),
        ],
        body=body,
    )


def build_gateway_response(
    status: int,
    reason: str,
    request: str,
    body: str = "",
    content_type: str | None = None,
    add_to_tag: bool = True,
) -> str:
    to_header = header_value(request, "To")
    if add_to_tag and ";tag=" not in to_header.lower():
        to_header = f"{to_header};tag={GATEWAY_TAG}"

    headers = [
        ("Via", header_value(request, "Via")),
        ("From", header_value(request, "From")),
        ("To", to_header),
        ("Call-ID", header_value(request, "Call-ID")),
        ("CSeq", header_value(request, "CSeq")),
        ("Contact", f"<sip:gw@{LOCAL_IP}>"),
    ]
    if content_type is not None:
        headers.append(("Content-Type", content_type))

    head = [f"SIP/2.0 {status} {reason}"]
    head.extend(f"{name}: {value}" for name, value in headers)
    head.append(f"Content-Length: {len(body.encode('utf-8'))}")
    return "\r\n".join(head) + "\r\n\r\n" + body


def sip_request(method: str, request_uri: str, headers: list[tuple[str, str]], body: str = "") -> str:
    head = [f"{method} {request_uri} SIP/2.0"]
    head.extend(f"{name}: {value}" for name, value in headers)
    head.append(f"Content-Length: {len(body.encode('utf-8'))}")
    return "\r\n".join(head) + "\r\n\r\n" + body


def sdp_body(origin: str, media_port: int, payload_types: tuple[int, ...] = (0, 8, 101)) -> str:
    payload_text = " ".join(str(payload_type) for payload_type in payload_types)
    rtpmap_lines = []
    if 0 in payload_types:
        rtpmap_lines.append("a=rtpmap:0 PCMU/8000\r\n")
    if 8 in payload_types:
        rtpmap_lines.append("a=rtpmap:8 PCMA/8000\r\n")
    if 101 in payload_types:
        rtpmap_lines.append("a=rtpmap:101 telephone-event/8000\r\n")
        rtpmap_lines.append("a=fmtp:101 0-16\r\n")

    return (
        "v=0\r\n"
        f"o={origin} 1 1 IN IP4 {LOCAL_IP}\r\n"
        f"s=VOS-RS full flow {origin}\r\n"
        f"c=IN IP4 {LOCAL_IP}\r\n"
        "t=0 0\r\n"
        f"m=audio {media_port} RTP/AVP {payload_text}\r\n"
        + "".join(rtpmap_lines)
    )


def rtp_packet(payload_type: int, sequence: int, timestamp: int, ssrc: int, payload: bytes) -> bytes:
    return (
        bytes([0x80, payload_type & 0x7F])
        + sequence.to_bytes(2, "big")
        + timestamp.to_bytes(4, "big")
        + ssrc.to_bytes(4, "big")
        + payload
    )


def rtcp_receiver_report(
    ssrc: int,
    source_ssrc: int,
    fraction_lost: int,
    cumulative_lost: int,
    jitter: int,
    last_sender_report: int,
    delay_since_last_sender_report: int,
) -> bytes:
    cumulative_lost &= 0xFFFFFF
    payload = (
        ssrc.to_bytes(4, "big")
        + source_ssrc.to_bytes(4, "big")
        + bytes([fraction_lost & 0xFF])
        + cumulative_lost.to_bytes(3, "big")
        + (0).to_bytes(4, "big")
        + jitter.to_bytes(4, "big")
        + last_sender_report.to_bytes(4, "big")
        + delay_since_last_sender_report.to_bytes(4, "big")
    )
    length_words = (4 + len(payload)) // 4 - 1
    return bytes([0x81, 201]) + length_words.to_bytes(2, "big") + payload


def recv_sip_text(udp_socket: socket.socket, label: str) -> str:
    data, _ = udp_socket.recvfrom(65535)
    text = data.decode("utf-8", errors="replace")
    if not text.startswith("SIP/2.0 "):
        raise FullFlowError(f"expected SIP response for {label}, got: {text.splitlines()[0]}")
    return text


def require_status(message: str, status: int) -> None:
    first_line = message.splitlines()[0] if message else ""
    if not first_line.startswith(f"SIP/2.0 {status} "):
        raise FullFlowError(f"expected SIP {status}, got {first_line}")


def header_value(message: str, name: str) -> str:
    needle = name.lower() + ":"
    for line in message.splitlines():
        if line.lower().startswith(needle):
            return line.split(":", 1)[1].strip()
    raise FullFlowError(f"missing SIP header: {name}")


def parse_sdp_endpoint(message: str) -> tuple[str, int]:
    _, body = split_sip_body(message)
    connection = re.search(r"(?im)^c=IN IP[46]\s+([^\r\n]+)", body)
    media = re.search(r"(?im)^m=audio\s+(\d+)\s+", body)
    if not connection or not media:
        raise FullFlowError("missing SDP RTP endpoint")
    return connection.group(1).strip(), int(media.group(1))


def require_audio_payloads(message: str, expected: set[str]) -> None:
    _, body = split_sip_body(message)
    media = re.search(r"(?im)^m=audio\s+\d+\s+\S+\s+([^\r\n]+)", body)
    if not media:
        raise FullFlowError("missing audio m-line in SDP")

    payloads = set(media.group(1).split())
    if not expected.issubset(payloads):
        raise FullFlowError(f"expected negotiated payloads {sorted(expected)} to be a subset of {sorted(payloads)}")


def split_sip_body(message: str) -> tuple[str, str]:
    if "\r\n\r\n" in message:
        return message.split("\r\n\r\n", 1)
    if "\n\n" in message:
        return message.split("\n\n", 1)
    return message, ""


def bind_udp(address: str, port: int) -> socket.socket:
    udp_socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    udp_socket.bind((address, port))
    return udp_socket


def bind_rtp_rtcp_pair(address: str) -> tuple[socket.socket, socket.socket]:
    candidates = list(range(30000, 62000, 2))
    random.shuffle(candidates)
    for port in candidates:
        rtp_socket: socket.socket | None = None
        rtcp_socket: socket.socket | None = None
        try:
            rtp_socket = bind_udp(address, port)
            rtcp_socket = bind_udp(address, port + 1)
            return rtp_socket, rtcp_socket
        except OSError:
            if rtp_socket is not None:
                rtp_socket.close()
            if rtcp_socket is not None:
                rtcp_socket.close()
    raise FullFlowError("could not find a free RTP/RTCP media port pair")


def unused_udp_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp_socket:
        udp_socket.bind((LOCAL_IP, 0))
        return udp_socket.getsockname()[1]


def unused_even_udp_pair() -> tuple[int, int]:
    candidates = list(range(42000, 52000, 2))
    random.shuffle(candidates)
    for port in candidates:
        sockets: list[socket.socket] = []
        try:
            for candidate in (port, port + 1, port + 2, port + 3):
                udp_socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                udp_socket.bind(("0.0.0.0", candidate))
                sockets.append(udp_socket)
            return port, port + 2
        except OSError:
            pass
        finally:
            for udp_socket in sockets:
                udp_socket.close()
    raise FullFlowError("could not find a free RTP relay port pair")


def wait_for_edge(edge_process: subprocess.Popen, edge_port: int) -> None:
    deadline = time.time() + TIMEOUT_SECONDS
    probe_socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    probe_socket.bind((LOCAL_IP, 0))
    probe_socket.settimeout(0.2)
    probe_port = probe_socket.getsockname()[1]
    probe = sip_request(
        method="OPTIONS",
        request_uri=f"sip:edge@{LOCAL_IP}:{edge_port}",
        headers=[
            ("Via", f"SIP/2.0/UDP {LOCAL_IP}:{probe_port};branch=z9hG4bK-full-flow-probe"),
            ("Max-Forwards", "70"),
            ("From", f"<sip:probe@{LOCAL_IP}:{probe_port}>;tag=probe"),
            ("To", f"<sip:edge@{LOCAL_IP}:{edge_port}>"),
            ("Call-ID", f"probe-{os.getpid()}@vos-rs.local"),
            ("CSeq", "1 OPTIONS"),
        ],
    ).encode("utf-8")

    while time.time() < deadline:
        if edge_process.poll() is not None:
            raise FullFlowError(f"sip-edge exited early with status {edge_process.returncode}")
        probe_socket.sendto(probe, (LOCAL_IP, edge_port))
        try:
            response, _ = probe_socket.recvfrom(65535)
            if response.startswith(b"SIP/2.0 200 "):
                probe_socket.close()
                return
        except socket.timeout:
            pass
        time.sleep(0.05)
    probe_socket.close()
    raise FullFlowError("sip-edge did not answer readiness OPTIONS")


def require_postgres() -> None:
    subprocess.run(["pg_isready"], cwd=ROOT_DIR, check=True, stdout=subprocess.DEVNULL)


def create_database(db_name: str) -> None:
    run_psql("postgres", f"DROP DATABASE IF EXISTS {db_name}")
    run_psql("postgres", f"CREATE DATABASE {db_name}")


def drop_database(db_name: str) -> None:
    if not db_name:
        return
    try:
        run_psql("postgres", f"DROP DATABASE IF EXISTS {db_name}")
    except Exception:
        pass


def wait_for_cdr(db_name: str, call_id: str) -> str:
    sql = (
        "SELECT call_id || ',' || status || ',' || COALESCE(caller, '') || ',' || "
        "COALESCE(callee, '') || ',' || billable_duration_ms || ',' || COALESCE(dtmf_digits, '') "
        f"FROM call_cdrs WHERE call_id = '{call_id}'"
    )
    deadline = time.time() + TIMEOUT_SECONDS
    last_output = ""
    while time.time() < deadline:
        result = run_psql_capture(db_name, sql)
        last_output = result.strip()
        if last_output:
            parts = last_output.split(",")
            if len(parts) >= 2 and parts[1] == "answered":
                return last_output
            raise FullFlowError(f"unexpected CDR row: {last_output}")
        time.sleep(0.2)
    raise FullFlowError(f"CDR row was not persisted; last output: {last_output}")


def run_psql(database: str, sql: str) -> None:
    subprocess.run(
        ["psql", "-d", database, "-v", "ON_ERROR_STOP=1", "-c", sql],
        cwd=ROOT_DIR,
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def run_psql_capture(database: str, sql: str) -> str:
    result = subprocess.run(
        ["psql", "-d", database, "-v", "ON_ERROR_STOP=1", "-At", "-c", sql],
        cwd=ROOT_DIR,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    return result.stdout


def terminate_process(process: subprocess.Popen) -> None:
    if process.poll() is not None:
        return
    process.send_signal(signal.SIGTERM)
    try:
        process.wait(timeout=2)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=2)


def write_log(name: str, content: str) -> None:
    (LOG_DIR / name).write_text(content, encoding="utf-8")


def print_failure_logs() -> None:
    for file_name in (
        "sip-edge.log",
        "caller_response_100.txt",
        "caller_response_180.txt",
        "caller_response_200.txt",
        "gateway_invite.txt",
        "gateway_ack.txt",
        "gateway_bye.txt",
    ):
        path = LOG_DIR / file_name
        if path.exists() and path.stat().st_size > 0:
            print(f"\n==> {path}", file=sys.stderr)
            print(tail_text(path), file=sys.stderr)


def tail_text(path: Path, max_lines: int = 80) -> str:
    return "\n".join(path.read_text(encoding="utf-8", errors="replace").splitlines()[-max_lines:])


if __name__ == "__main__":
    raise SystemExit(main())
