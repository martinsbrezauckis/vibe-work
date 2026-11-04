//! SOCKS4/4a and SOCKS5 handshakes over a transport. Called by transport with checked IPs only.
//! Remote DNS aliases deliberately send IP address types; SOCKS4 cannot carry IPv6 or password authentication.
use super::{Error, Route};
use crate::stream::Stream;
use std::net::{IpAddr, SocketAddr};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn write(stream: &mut Stream, bytes: &[u8]) -> Result<(), Error> {
    stream
        .write_all(bytes)
        .await
        .map_err(|_| Error::Unreachable)
}
async fn read<const N: usize>(stream: &mut Stream) -> Result<[u8; N], Error> {
    let mut bytes = [0; N];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(|_| Error::Protocol)?;
    Ok(bytes)
}

pub(super) async fn v4(
    stream: &mut Stream,
    route: &Route,
    target: SocketAddr,
) -> Result<(), Error> {
    let IpAddr::V4(ip) = target.ip() else {
        return Err(Error::Configuration);
    };
    let user = if let Some((user, password)) = &route.credentials {
        // SOCKS4's USERID is not password authentication. Never silently discard a configured password.
        if !password.is_empty() || user.contains('\0') || user.len() > 255 {
            return Err(Error::Authentication);
        }
        user.as_bytes()
    } else {
        &[]
    };
    let mut request = vec![4, 1];
    request.extend_from_slice(&target.port().to_be_bytes());
    request.extend_from_slice(&ip.octets());
    request.extend_from_slice(user);
    request.push(0);
    write(stream, &request).await?;
    let answer = read::<8>(stream).await?;
    if answer[0] != 0 {
        return Err(Error::Protocol);
    }
    match answer[1] {
        90 => Ok(()),
        92 | 93 => Err(Error::Authentication),
        91 => Err(Error::Refused),
        _ => Err(Error::Protocol),
    }
}

pub(super) async fn v5(
    stream: &mut Stream,
    route: &Route,
    target: SocketAddr,
) -> Result<(), Error> {
    // With configured credentials offer ONLY username/password. A server cannot silently downgrade sign-in.
    let method = if route.credentials.is_some() { 2 } else { 0 };
    write(stream, &[5, 1, method]).await?;
    if read::<2>(stream).await? != [5, method] {
        return Err(Error::Authentication);
    }
    if let Some((user, password)) = &route.credentials {
        let u = u8::try_from(user.len())
            .ok()
            .filter(|n| *n > 0)
            .ok_or(Error::Authentication)?;
        let p = u8::try_from(password.len())
            .ok()
            .filter(|n| *n > 0)
            .ok_or(Error::Authentication)?;
        let mut auth = vec![1, u];
        auth.extend_from_slice(user.as_bytes());
        auth.push(p);
        auth.extend_from_slice(password.as_bytes());
        write(stream, &auth).await?;
        if read::<2>(stream).await? != [1, 0] {
            return Err(Error::Authentication);
        }
    }
    let mut request = vec![5, 1, 0];
    match target.ip() {
        IpAddr::V4(ip) => {
            request.push(1);
            request.extend_from_slice(&ip.octets());
        }
        IpAddr::V6(ip) => {
            request.push(4);
            request.extend_from_slice(&ip.octets());
        }
    }
    request.extend_from_slice(&target.port().to_be_bytes());
    write(stream, &request).await?;
    let answer = read::<4>(stream).await?;
    if answer[0] != 5 || answer[2] != 0 {
        return Err(Error::Protocol);
    }
    if answer[1] != 0 {
        return Err(Error::Refused);
    }
    let length = match answer[3] {
        1 => 4,
        4 => 16,
        3 => usize::from(read::<1>(stream).await?[0]),
        _ => return Err(Error::Protocol),
    };
    let mut bound = vec![0; length + 2];
    stream
        .read_exact(&mut bound)
        .await
        .map_err(|_| Error::Protocol)?;
    Ok(())
}
